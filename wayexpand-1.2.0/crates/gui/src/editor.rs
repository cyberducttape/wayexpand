use anyhow::{Context, Result};
use wayexpand_core::{CommandConfig, CommandEnvironment, ExpansionConfig, MatchMode};

/// Editable form state for one expansion. Keeping this separate from the
/// application shell makes editor validation and future editor widgets
/// independently testable.
pub(crate) struct Draft {
    pub(crate) trigger: String,
    pub(crate) description: String,
    pub(crate) tags: String,
    pub(crate) category: String,
    pub(crate) app_filter: String,
    pub(crate) replacement: String,
    pub(crate) enabled: bool,
    pub(crate) match_mode: MatchMode,
    pub(crate) propagate_case: bool,
    pub(crate) command_enabled: bool,
    pub(crate) command_program: String,
    pub(crate) command_args: String,
    pub(crate) command_timeout_ms: String,
    pub(crate) command_cache_ms: String,
}

impl Draft {
    pub(crate) fn from_expansion(expansion: &ExpansionConfig) -> Self {
        let (command_enabled, command_program, command_args, command_timeout_ms, command_cache_ms) =
            match &expansion.command {
                Some(command) => (
                    true,
                    command.program.clone(),
                    command.args.join("\n"),
                    command.timeout_ms.to_string(),
                    command.cache_ms.to_string(),
                ),
                None => (
                    false,
                    String::new(),
                    String::new(),
                    "500".into(),
                    "0".into(),
                ),
            };
        Self {
            trigger: expansion.trigger.clone(),
            description: expansion.description.clone(),
            tags: expansion.tags.join(", "),
            category: expansion.category.clone(),
            app_filter: expansion.app_filter.join(", "),
            replacement: expansion.replacement.clone(),
            enabled: expansion.enabled,
            match_mode: expansion.match_mode,
            propagate_case: expansion.propagate_case,
            command_enabled,
            command_program,
            command_args,
            command_timeout_ms,
            command_cache_ms,
        }
    }

    pub(crate) fn command_config(&self) -> Result<Option<CommandConfig>> {
        if !self.command_enabled {
            return Ok(None);
        }
        let program = self.command_program.trim();
        if program.is_empty() {
            anyhow::bail!("program is required when command expansion is enabled");
        }
        let timeout_ms = self
            .command_timeout_ms
            .trim()
            .parse::<u64>()
            .context("timeout must be an integer in milliseconds")?;
        let cache_ms = self
            .command_cache_ms
            .trim()
            .parse::<u64>()
            .context("cache duration must be an integer in milliseconds")?;
        Ok(Some(CommandConfig {
            program: program.to_owned(),
            args: self
                .command_args
                .lines()
                .map(str::trim)
                .filter(|arg| !arg.is_empty())
                .map(str::to_owned)
                .collect(),
            timeout_ms,
            cache_ms,
            environment: CommandEnvironment::default(),
            pass_env: Vec::new(),
        }))
    }
}
