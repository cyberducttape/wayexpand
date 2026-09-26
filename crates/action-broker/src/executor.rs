//! Action Executor - Safe command execution with policy enforcement.
//!
//! Executes actions within the constraints defined in ActionConfig and BrokerConfig.

use crate::{
    config::BrokerConfig,
    protocol::{ActionError, ActionOutput, ActionRequest, ActionResponse},
};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::Instant;

pub struct ActionExecutor {
    config: BrokerConfig,
}

impl ActionExecutor {
    pub fn new(config: &BrokerConfig) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate()?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Execute an action synchronously with timeout and policy enforcement.
    pub async fn execute(&self, request: ActionRequest) -> Result<ActionResponse, ActionError> {
        // Validate the action exists and is enabled
        let action_config =
            self.config
                .get_action(&request.action_id)
                .ok_or(ActionError::ActionNotFound {
                    action_id: request.action_id.clone(),
                })?;

        if !action_config.is_enabled() {
            return Err(ActionError::ActionBlocked {
                action_id: request.action_id.clone(),
                reason: "action is disabled".to_string(),
            });
        }

        // Build the command with policy enforcement
        let mut cmd = Command::new(&action_config.program);

        // Add prefix arguments (immutable)
        for arg in &action_config.args_prefix {
            cmd.arg(arg);
        }

        // Set working directory
        if let Some(cwd) = &action_config.cwd {
            cmd.current_dir(cwd);
        } else if let Some(default_cwd) = &self.config.default_cwd {
            cmd.current_dir(default_cwd);
        }

        // Set environment variables with policy enforcement
        cmd.env_clear(); // Start fresh

        if request.inherit_env && !self.config.strict_env {
            // Inherit environment but filter with allowed list if specified
            if action_config.pass_env.is_empty() {
                // Allow all if no restrictions
                for (key, value) in std::env::vars() {
                    cmd.env(&key, &value);
                }
            } else {
                // Allow only specified variables
                for key in &action_config.pass_env {
                    if let Ok(value) = std::env::var(key) {
                        cmd.env(key, value);
                    }
                }
            }
        } else {
            // Use only explicitly passed variables
            let allowed_vars = self.build_env_map(&request.env_vars, &action_config.pass_env);
            for (key, value) in allowed_vars {
                cmd.env(&key, value);
            }
        }

        // Configure output capture
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Spawn and execute with timeout
        let start = Instant::now();

        let child = cmd.spawn().map_err(|e| ActionError::SpawnFailed {
            action_id: request.action_id.clone(),
            program: action_config.program.clone(),
            reason: e.to_string(),
        })?;

        // Wait with timeout using blocking task
        let timeout = request.timeout();
        let action_id = request.action_id.clone();

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || child.wait_with_output())
                .await
                .map_err(|e| ActionError::Internal {
                    reason: format!("task join error: {}", e),
                })
                .and_then(|res| {
                    res.map_err(|e| ActionError::Internal {
                        reason: format!("child process error: {}", e),
                    })
                })
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if exit_code == 0 {
                    Ok(ActionResponse::Success(ActionOutput {
                        exit_code,
                        stdout,
                        stderr,
                        duration_ms,
                    }))
                } else {
                    Err(ActionError::ExitFailure {
                        action_id: request.action_id.clone(),
                        exit_code,
                        stderr: stderr.trim().to_string(),
                    })
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ActionError::Timeout {
                action_id,
                timeout_ms: timeout.as_millis() as u64,
            }),
        }
    }

    /// Build environment map from request variables and allowed list.
    fn build_env_map(
        &self,
        request_vars: &[String],
        allowed_vars: &[String],
    ) -> HashMap<String, String> {
        let mut map = HashMap::new();

        // Parse request variables (format: KEY=value)
        for var_str in request_vars {
            if let Some((key, value)) = var_str.split_once('=') {
                if allowed_vars.is_empty() || allowed_vars.contains(&key.to_string()) {
                    map.insert(key.to_string(), value.to_string());
                }
            }
        }

        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ActionConfig;

    #[tokio::test]
    async fn executor_action_not_found() {
        let config = BrokerConfig::default();
        let executor = ActionExecutor::new(&config).unwrap();

        let request = ActionRequest {
            action_id: "nonexistent".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::ActionNotFound { .. })));
    }

    #[tokio::test]
    async fn executor_disabled_action() {
        let mut config = BrokerConfig::default();
        config.actions.insert(
            "disabled".to_string(),
            ActionConfig {
                program: "/bin/echo".to_string(),
                args_prefix: vec![],
                timeout_ms: 5000,
                allow_network: false,
                pass_env: vec![],
                inherit_env: false,
                cwd: None,
                enabled: false,
                description: None,
            },
        );

        let executor = ActionExecutor::new(&config).unwrap();
        let request = ActionRequest {
            action_id: "disabled".to_string(),
            timeout_ms: 5000,
            inherit_env: false,
            env_vars: vec![],
            stdout_capture: true,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ActionError::ActionBlocked { .. })));
    }

    #[test]
    fn executor_env_map_building() {
        let config = BrokerConfig::default();
        let executor = ActionExecutor::new(&config).unwrap();

        let request_vars = vec![
            "HOME=/home/user".to_string(),
            "SECRET=should_be_filtered".to_string(),
        ];
        let allowed_vars = vec!["HOME".to_string()];

        let map = executor.build_env_map(&request_vars, &allowed_vars);
        assert_eq!(map.get("HOME"), Some(&"/home/user".to_string()));
        assert!(!map.contains_key("SECRET"));
    }
}
