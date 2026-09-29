//! Action Broker Configuration - Action definitions and policies.
//!
//! Defines the configuration schema for available actions and their permissions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionConfig {
    /// The command to execute.
    pub program: String,

    /// Arguments to pass to the program (prefix for pattern matching).
    #[serde(default)]
    pub args_prefix: Vec<String>,

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
        for (id, action) in &self.actions {
            action
                .validate()
                .map_err(|e| format!("action '{}': {}", id, e))?;

            if self.require_absolute_paths && !action.program.starts_with('/') {
                return Err(format!(
                    "action '{}': program must be absolute path when require_absolute_paths=true",
                    id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_config_validation() {
        let config = ActionConfig {
            program: "/usr/bin/kubectl".to_string(),
            args_prefix: vec!["get".to_string()],
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
            args_prefix: vec![],
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
args_prefix = ["hello"]
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
                args_prefix: vec![],
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
args_prefix = []
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
}
