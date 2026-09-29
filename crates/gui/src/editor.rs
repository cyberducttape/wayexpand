use anyhow::{Context, Result};
use wayexpand_core::{CommandConfig, CommandEnvironment, ExpansionConfig, MatchMode};

const MAX_COMMAND_ARGS: usize = 32;
const MAX_COMMAND_PROGRAM_CHARS: usize = 256;
const MAX_COMMAND_ARG_CHARS: usize = 1024;
const MAX_COMMAND_ARG_DATA_CHARS: usize = 16 * 1024;
const MAX_COMMAND_TIMEOUT_MS: u64 = 5_000;
const MAX_COMMAND_CACHE_MS: u64 = 60_000;

/// Editable form state for one expansion. Keeping this separate from the
/// application shell makes editor validation and future editor widgets
/// independently testable.
#[derive(Clone)]
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
        let args: Vec<String> = self
            .command_args
            .lines()
            .map(str::trim)
            .filter(|arg| !arg.is_empty())
            .map(str::to_owned)
            .collect();
        if program.chars().count() > MAX_COMMAND_PROGRAM_CHARS {
            anyhow::bail!("program is too long");
        }
        if program.contains('\0') {
            anyhow::bail!("program cannot contain NUL bytes");
        }
        if args.len() > MAX_COMMAND_ARGS {
            anyhow::bail!("too many command arguments");
        }
        let arg_data_chars: usize = args.iter().map(|arg| arg.chars().count()).sum();
        if arg_data_chars > MAX_COMMAND_ARG_DATA_CHARS {
            anyhow::bail!("command arguments are too large");
        }
        if args
            .iter()
            .any(|arg| arg.chars().count() > MAX_COMMAND_ARG_CHARS || arg.contains('\0'))
        {
            anyhow::bail!("command argument is too long or contains NUL bytes");
        }
        if !(1..=MAX_COMMAND_TIMEOUT_MS).contains(&timeout_ms) {
            anyhow::bail!("timeout must be between 1 and 5000 milliseconds");
        }
        if cache_ms > MAX_COMMAND_CACHE_MS {
            anyhow::bail!("cache duration must not exceed 60000 milliseconds");
        }
        Ok(Some(CommandConfig {
            program: program.to_owned(),
            args,
            timeout_ms,
            cache_ms,
            environment: CommandEnvironment::default(),
            pass_env: Vec::new(),
        }))
    }
}
