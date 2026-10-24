use anyhow::{Context, Result};
use wayexpand_core::{
    validate_command_config, CommandConfig, CommandEnvironment, ExpansionConfig, MatchMode,
};

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
    /// `output_file` for `wayexpand new`; empty means none.
    pub(crate) output_file: String,
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
            output_file: expansion.output_file.clone().unwrap_or_default(),
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
            output_file: Some(self.output_file.trim().to_owned()).filter(|file| !file.is_empty()),
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
        // Limits are enforced by core's validate_command_config below, the
        // same check the daemon and CLI apply, so the editor cannot drift.
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
///
/// The picker asks on every frame it is shown, so the answer is cached for
/// [`BROKER_ACTIONS_REFRESH`]; edits to the policy appear within that time.
pub(crate) fn broker_action_ids() -> std::sync::Arc<[String]> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, std::sync::Arc<[String]>)>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((read_at, ids)) = cache.as_ref() {
        if read_at.elapsed() < BROKER_ACTIONS_REFRESH {
            return std::sync::Arc::clone(ids);
        }
    }
    let ids: std::sync::Arc<[String]> = read_broker_action_ids(&broker_config_path()).into();
    *cache = Some((std::time::Instant::now(), std::sync::Arc::clone(&ids)));
    ids
}

const BROKER_ACTIONS_REFRESH: std::time::Duration = std::time::Duration::from_secs(2);
/// Far above any real broker policy; anything larger is not read.
const MAX_BROKER_CONFIG_BYTES: u64 = 1024 * 1024;

fn broker_config_path() -> std::path::PathBuf {
    std::env::var_os("WAYEXPAND_ACTION_BROKER_CONFIG")
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
        })
}

fn read_broker_action_ids(path: &std::path::Path) -> Vec<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // Non-blocking, so a FIFO at the path cannot stall the UI thread; only
    // a bounded regular file is read.
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    else {
        return Vec::new();
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_BROKER_CONFIG_BYTES)
    {
        return Vec::new();
    }
    let mut source = String::new();
    if file
        .take(MAX_BROKER_CONFIG_BYTES)
        .read_to_string(&mut source)
        .is_err()
    {
        return Vec::new();
    }
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

#[cfg(test)]
mod broker_policy_tests {
    use super::read_broker_action_ids;

    #[test]
    fn broker_action_ids_read_a_policy_and_never_block_on_a_fifo() {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-gui-broker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let policy = directory.join("broker.toml");
        std::fs::write(
            &policy,
            "[actions.restart-web]\nprogram = \"/bin/true\"\n[actions.backup]\nprogram = \"/bin/true\"\n",
        )
        .unwrap();
        assert_eq!(read_broker_action_ids(&policy), ["backup", "restart-web"]);

        let fifo = directory.join("fifo.toml");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(read_broker_action_ids(&fifo).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
