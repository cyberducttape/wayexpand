//! Action Broker Configuration - Action definitions and policies.
//!
//! Defines the configuration schema for available actions and their permissions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionConfig {
    /// The command to execute.
    pub program: String,

    /// Arguments to pass to the program (prefix for pattern matching).
    #[serde(default)]
    pub args_prefix: Vec<String>,

    /// Maximum execution time in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Whether to allow network access (AF_INET, AF_INET6).
    #[serde(default)]
    pub allow_network: bool,

    /// Environment variables to pass to the action.
    #[serde(default)]
    pub pass_env: Vec<String>,

    /// Whether to inherit all environment variables from daemon.
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
        if !self.program.starts_with('/') && !self.program.contains('/') {
            // Allow both absolute paths and simple program names (will be resolved from PATH)
        }
        Ok(())
    }
}

/// Complete broker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    /// Configured actions keyed by action_id.
    #[serde(default)]
    pub actions: HashMap<String, ActionConfig>,

    /// Whether to require absolute command paths.
    /// If true, only /absolute/path/to/program is allowed.
    #[serde(default)]
    pub require_absolute_paths: bool,

    /// Whether to enforce that only explicitly allowed environment variables are passed.
    #[serde(default = "default_strict_env")]
    pub strict_env: bool,

    /// Default working directory if action doesn't specify one.
    #[serde(default)]
    pub default_cwd: Option<String>,

    /// Enable audit logging of all action executions.
    #[serde(default)]
    pub audit_enabled: bool,

    /// Optional audit log path (e.g., /var/log/wayexpand-actions.log).
    #[serde(default)]
    pub audit_path: Option<String>,
}

fn default_strict_env() -> bool {
    true
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            actions: HashMap::new(),
            require_absolute_paths: false,
            strict_env: true,
            default_cwd: None,
            audit_enabled: false,
            audit_path: None,
        }
    }
}

impl BrokerConfig {
    /// Load configuration from TOML file.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
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
            allow_network: true,
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
            allow_network: false,
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
require_absolute_paths = true
strict_env = true

[actions."test_action"]
program = "/usr/bin/echo"
args_prefix = ["hello"]
timeout_ms = 5000
allow_network = false
"#;
        let config = BrokerConfig::from_toml(toml_str).unwrap();
        assert!(config.require_absolute_paths);
        assert!(config.get_action("test_action").is_some());
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
                allow_network: false,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: true,
                description: None,
            },
        );
        assert!(config.validate().is_err());
    }
}
