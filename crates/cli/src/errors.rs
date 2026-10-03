//! Error classification: typed CLI error categories and their stable exit codes.

use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliErrorKind {
    Usage,
    Config,
    Daemon,
    Operational,
}

#[derive(Debug)]
pub(crate) struct CliError {
    kind: CliErrorKind,
    message: String,
}

pub(crate) fn classified_error(kind: CliErrorKind, message: impl Into<String>) -> Error {
    Error::new(CliError {
        kind,
        message: message.into(),
    })
}

/// Map a configuration load failure to a CLI error. A missing file is the
/// usual first-run state, so it gets an actionable message naming the path
/// the user (or the default) chose; everything else goes through
/// `safe_summary` so snippet content never reaches the terminal.
pub(crate) fn config_load_error(path: &Path, error: wayexpand_core::ConfigError) -> Error {
    if let wayexpand_core::ConfigError::Read { source, .. } = &error {
        if source.kind() == io::ErrorKind::NotFound {
            return config_error(format!(
                "no configuration file at {}; create one with `wayexpand-gui` or `wayexpand-ui`",
                path.display()
            ));
        }
    }
    config_error(format!("configuration invalid: {}", error.safe_summary()))
}

pub(crate) fn usage_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Usage, message)
}

pub(crate) fn config_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Config, message)
}

pub(crate) fn daemon_error(message: impl Into<String>) -> Error {
    classified_error(CliErrorKind::Daemon, message)
}

pub(crate) fn exit_code_for(error: &anyhow::Error) -> i32 {
    match error.downcast_ref::<CliError>().map(|error| error.kind) {
        Some(CliErrorKind::Usage) => EXIT_USAGE,
        Some(CliErrorKind::Config) => EXIT_CONFIG,
        Some(CliErrorKind::Daemon) => EXIT_DAEMON,
        Some(CliErrorKind::Operational) | None => 1,
    }
}

pub(crate) fn normalize_error(error: Error) -> Error {
    if error.downcast_ref::<CliError>().is_some() {
        error
    } else {
        // `{:#}` keeps the cause chain ("creating backup X: File exists").
        // `to_string()` kept only the outermost context, so every
        // `.context(...)` failure lost the part that explained it.
        classified_error(CliErrorKind::Operational, format!("{error:#}"))
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for CliError {}
