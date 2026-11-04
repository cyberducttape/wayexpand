//! Explicit context reported by an opted-in terminal integration.

use serde::{Deserialize, Serialize};

/// A bounded report from one local shell session. The daemon keys it to the
/// controlling terminal device and random per-shell session identifier.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionContextReport {
    pub terminal_id: String,
    pub session_id: String,
    pub shell: Option<String>,
    pub remote_host: Option<String>,
    pub environment: Option<String>,
    pub project_root: Option<String>,
    pub kubernetes_context: Option<String>,
    pub container: Option<String>,
}

impl SessionContextReport {
    /// Validate identifiers and cap the complete wire representation before
    /// sending it over the bounded daemon control socket.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.terminal_id.is_empty()
            || self.terminal_id.len() > 96
            || !self
                .terminal_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte))
        {
            return Err("terminal identity is invalid");
        }
        if self.session_id.len() != 32
            || !self.session_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("session identifier must contain 32 hexadecimal characters");
        }
        for (value, maximum, name) in [
            (self.shell.as_deref(), 32, "shell"),
            (self.remote_host.as_deref(), 253, "remote host"),
            (self.environment.as_deref(), 64, "environment"),
            (self.project_root.as_deref(), 512, "project root"),
            (
                self.kubernetes_context.as_deref(),
                256,
                "Kubernetes context",
            ),
            (self.container.as_deref(), 128, "container"),
        ] {
            if value.is_some_and(|value| {
                value.is_empty() || value.len() > maximum || value.chars().any(char::is_control)
            }) {
                return Err(match name {
                    "shell" => "shell value is too long or contains control characters",
                    "remote host" => "remote host is too long or contains control characters",
                    "environment" => "environment is too long or contains control characters",
                    "project root" => "project root is too long or contains control characters",
                    "Kubernetes context" => {
                        "Kubernetes context is too long or contains control characters"
                    }
                    _ => "container value is too long or contains control characters",
                });
            }
        }
        if self
            .project_root
            .as_deref()
            .is_some_and(|path| !std::path::Path::new(path).is_absolute())
        {
            return Err("project root must be an absolute path");
        }
        let encoded =
            serde_json::to_vec(self).map_err(|_| "context report could not be encoded")?;
        if encoded.len() > 1024 {
            return Err("context report exceeds the control message size limit");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> SessionContextReport {
        SessionContextReport {
            terminal_id: "tty-1-2".into(),
            session_id: "0123456789abcdef0123456789abcdef".into(),
            shell: Some("bash".into()),
            remote_host: Some("web-staging".into()),
            environment: Some("staging".into()),
            ..SessionContextReport::default()
        }
    }

    #[test]
    fn explicit_shell_and_remote_context_validates() {
        assert_eq!(report().validate(), Ok(()));
    }

    #[test]
    fn empty_or_control_bearing_context_labels_are_rejected() {
        let mut invalid = report();
        invalid.environment = Some(String::new());
        assert_eq!(
            invalid.validate(),
            Err("environment is too long or contains control characters")
        );

        let mut invalid = report();
        invalid.remote_host = Some("prod\nwrong-host".into());
        assert_eq!(
            invalid.validate(),
            Err("remote host is too long or contains control characters")
        );
    }
}
