use anyhow::{Context, Result};
use wayexpand_core::{
    validate_command_config, CommandConfig, CommandEnvironment, ExpansionConfig, MatchMode,
};

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
    pub(crate) aliases: Vec<String>,
    pub(crate) description: String,
    pub(crate) tags: Vec<String>,
    pub(crate) category: String,
    pub(crate) app_filter: Vec<String>,
    pub(crate) replacement: String,
    pub(crate) enabled: bool,
    pub(crate) match_mode: MatchMode,
    pub(crate) propagate_case: bool,
    pub(crate) command_enabled: bool,
    pub(crate) command_action_mode: bool,
    pub(crate) command_action: String,
    pub(crate) command_program: String,
    pub(crate) command_args: Vec<String>,
    pub(crate) command_timeout_ms: String,
    pub(crate) command_cache_ms: String,
    pub(crate) command_environment: CommandEnvironment,
    pub(crate) command_pass_env: Vec<String>,
    /// Text typed into the "add tag" input but not yet confirmed with Enter.
    /// It counts as part of the draft so Save never drops a half-typed tag.
    pub(crate) pending_tag: String,
    /// Same as `pending_tag`, for the "add app" input.
    pub(crate) pending_app: String,
    /// Same as `pending_tag`, for the "add alias" input.
    pub(crate) pending_alias: String,
}

impl Draft {
    pub(crate) fn from_expansion(expansion: &ExpansionConfig) -> Self {
        let (
            command_enabled,
            command_action_mode,
            command_action,
            command_program,
            command_args,
            command_timeout_ms,
            command_cache_ms,
            command_environment,
            command_pass_env,
        ) = match &expansion.command {
            Some(command) => (
                true,
                command.action.is_some(),
                command.action.clone().unwrap_or_default(),
                command.program.clone(),
                command.args.clone(),
                command.timeout_ms.to_string(),
                command.cache_ms.to_string(),
                command.environment,
                command.pass_env.clone(),
            ),
            None => (
                false,
                false,
                String::new(),
                String::new(),
                Vec::new(),
                "500".into(),
                "0".into(),
                CommandEnvironment::default(),
                Vec::new(),
            ),
        };
        Self {
            trigger: expansion.trigger.clone(),
            aliases: expansion.aliases.clone(),
            description: expansion.description.clone(),
            tags: expansion.tags.clone(),
            category: expansion.category.clone(),
            app_filter: expansion.app_filter.clone(),
            replacement: expansion.replacement.clone(),
            enabled: expansion.enabled,
            match_mode: expansion.match_mode,
            propagate_case: expansion.propagate_case,
            command_enabled,
            command_action_mode,
            command_action,
            command_program,
            command_args,
            command_timeout_ms,
            command_cache_ms,
            command_environment,
            command_pass_env,
            pending_tag: String::new(),
            pending_app: String::new(),
            pending_alias: String::new(),
        }
    }

    /// Tags as they will be saved: the confirmed chips plus any pending input.
    pub(crate) fn committed_tags(&self) -> Vec<String> {
        with_pending_token(&self.tags, &self.pending_tag)
    }

    /// Aliases as they will be saved, including any pending input.
    pub(crate) fn committed_aliases(&self) -> Vec<String> {
        with_pending_token(&self.aliases, &self.pending_alias)
    }

    /// App filters as they will be saved, including any pending input.
    pub(crate) fn committed_app_filter(&self) -> Vec<String> {
        with_pending_token(&self.app_filter, &self.pending_app)
    }

    /// The expansion this draft describes, exactly as Save would store it
    /// (pending tag/alias/app input included). Preview and both save paths
    /// use this one conversion so a field cannot be forgotten in one of them.
    pub(crate) fn to_expansion(&self, id: String) -> Result<ExpansionConfig> {
        Ok(ExpansionConfig {
            id,
            trigger: self.trigger.clone(),
            replacement: self.replacement.clone(),
            description: self.description.clone(),
            tags: self.committed_tags(),
            category: self.category.clone(),
            app_filter: self.committed_app_filter(),
            match_mode: self.match_mode,
            command: self.command_config()?,
            enabled: self.enabled,
            propagate_case: self.propagate_case,
            aliases: self.committed_aliases(),
        })
    }

    /// Compare the raw form fields with the loaded model, without attempting
    /// to validate or normalize them. Invalid edits must still be dirty.
    pub(crate) fn matches_command(&self, command: Option<&CommandConfig>) -> bool {
        match command {
            Some(command) => {
                self.command_enabled
                    && self.command_action == command.action.clone().unwrap_or_default()
                    && self.command_action_mode == command.action.is_some()
                    && self.command_program == command.program
                    && self.command_args == command.args
                    && self.command_timeout_ms == command.timeout_ms.to_string()
                    && self.command_cache_ms == command.cache_ms.to_string()
                    && self.command_environment == command.environment
                    && self.command_pass_env == command.pass_env
            }
            None => {
                !self.command_enabled
                    && self.command_program.is_empty()
                    && self.command_action.is_empty()
                    && !self.command_action_mode
                    && self.command_args.is_empty()
                    && self.command_timeout_ms == "500"
                    && self.command_cache_ms == "0"
                    && self.command_environment == CommandEnvironment::default()
                    && self.command_pass_env.is_empty()
            }
        }
    }

    pub(crate) fn command_config(&self) -> Result<Option<CommandConfig>> {
        if !self.command_enabled {
            return Ok(None);
        }
        let action = self.command_action.trim();
        let program = self.command_program.as_str();
        if self.command_action_mode && action.is_empty() {
            anyhow::bail!("a broker action ID is required");
        }
        if !self.command_action_mode && program.trim().is_empty() {
            anyhow::bail!("program is required for direct execution");
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
        let args = self.command_args.clone();
        if action.chars().count() > MAX_COMMAND_PROGRAM_CHARS {
            anyhow::bail!("action ID is too long");
        }
        if action.contains('\0') {
            anyhow::bail!("action ID cannot contain NUL bytes");
        }
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
        let command = CommandConfig {
            action: self.command_action_mode.then(|| action.to_owned()),
            program: if self.command_action_mode {
                String::new()
            } else {
                program.to_owned()
            },
            args,
            timeout_ms,
            cache_ms,
            environment: self.command_environment,
            pass_env: self.command_pass_env.clone(),
        };
        validate_command_config(&command).map_err(anyhow::Error::msg)?;
        Ok(Some(command))
    }
}

/// Return action IDs from the operator's broker policy for the editor's
/// picker. The text field remains editable so a policy can be prepared before
/// the broker is installed or while a fleet-managed policy is being rolled out.
pub(crate) fn broker_action_ids() -> Vec<String> {
    let path = std::env::var_os("WAYEXPAND_ACTION_BROKER_CONFIG")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|dir| {
                std::path::PathBuf::from(dir)
                    .join("wayexpand")
                    .join("broker.toml")
            })
        })
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config/wayexpand/broker.toml")
        });
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(document) = source.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    let Some(actions) = document.get("actions").and_then(toml_edit::Item::as_table) else {
        return Vec::new();
    };
    let mut ids = actions
        .iter()
        .map(|(id, _)| id.to_owned())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// `tokens` plus the trimmed `pending` entry, unless it is empty or already
/// present. Shared by the tag and app-filter chip editors.
pub(crate) fn with_pending_token(tokens: &[String], pending: &str) -> Vec<String> {
    let mut committed = tokens.to_vec();
    let pending = pending.trim();
    if !pending.is_empty() && !committed.iter().any(|token| token == pending) {
        committed.push(pending.to_owned());
    }
    committed
}
