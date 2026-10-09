use crate::{
    render_template_with_cursor, ClipboardReader, KeyChord, TemplateContext, TemplateError,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

use crate::limits::COMMAND_LIMITS;
use crate::limits::{
    MAX_ALIASES, MAX_APP_FILTERS, MAX_APP_FILTER_CHARS, MAX_CATEGORY_CHARS, MAX_DESCRIPTION_CHARS,
    MAX_EXPANSIONS, MAX_HOTKEYS, MAX_HOTKEY_DESCRIPTION_CHARS, MAX_REPLACEMENT_BYTES, MAX_TAGS,
    MAX_TAG_CHARS, MAX_TEMPLATE_ENV, MAX_TRIGGER_CHARS,
};
pub(crate) use crate::limits::{
    MAX_CONFIG_BYTES, MAX_EFFECTIVE_TRIGGERS, MAX_EFFECTIVE_TRIGGER_SCALARS,
    MAX_TOTAL_TRIGGER_CHARS,
};
const MAX_COMMAND_ARGS: usize = COMMAND_LIMITS.max_args;
const MAX_COMMAND_PROGRAM_CHARS: usize = COMMAND_LIMITS.max_program_chars;
const MAX_COMMAND_ARG_CHARS: usize = COMMAND_LIMITS.max_arg_chars;
const MAX_COMMAND_ARG_DATA_CHARS: usize = COMMAND_LIMITS.max_arg_data_chars;
const MAX_COMMAND_ENV_VARS: usize = COMMAND_LIMITS.max_env_vars;
const MAX_COMMAND_ENV_NAME_CHARS: usize = COMMAND_LIMITS.max_env_name_chars;
const MAX_COMMAND_TIMEOUT_MS: u64 = COMMAND_LIMITS.max_timeout_ms;
const MAX_COMMAND_CACHE_MS: u64 = COMMAND_LIMITS.max_cache_ms;
const CONFIG_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
static EXPANSION_ID_FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(1);

fn new_expansion_id() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = EXPANSION_ID_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed);
        bytes[..8].copy_from_slice(&(timestamp as u64).to_be_bytes());
        bytes[8..12].copy_from_slice(&std::process::id().to_be_bytes());
        bytes[12..].copy_from_slice(&(counter as u32).to_be_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut hex = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            hex.push('-');
        }
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn source_has_legacy_expansion_ids(source: &str) -> bool {
    let Ok(document) = toml::from_str::<toml::Value>(source) else {
        return false;
    };
    document
        .get("expansion")
        .and_then(toml::Value::as_array)
        .is_some_and(|expansions| {
            expansions.iter().any(|expansion| {
                expansion
                    .as_table()
                    .is_some_and(|table| !table.contains_key("id"))
            })
        })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub expansion: Vec<ExpansionConfig>,
    #[serde(default)]
    pub hotkey: Vec<HotkeyConfig>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub organization: OrganizationPolicy,
}

/// Exact validated source snapshot used to detect a stale editor before save.
/// The source is shared with `LoadedConfig` so format-preserving editors can
/// parse the same bytes without reopening the path.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigRevision(Arc<str>);

impl std::fmt::Debug for ConfigRevision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigRevision")
            .field("bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub revision: ConfigRevision,
    source: Arc<str>,
}

impl LoadedConfig {
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A keyboard chord which invokes a bounded direct program action.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HotkeyConfig {
    pub chord: String,
    #[serde(default)]
    pub description: String,
    pub command: CommandConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub max_buffer_chars: usize,
    /// A key chord (e.g. `"Ctrl+Z"`) that, pressed immediately after a
    /// successful expansion with no other keystroke in between, reverts it
    /// -- erasing the inserted replacement and typing the original trigger
    /// back. `None` (the default) disables this entirely, matching every
    /// config written before it existed.
    pub undo_chord: Option<String>,
    /// Font size scaling for the GUI (Small, Normal, Large, ExtraLarge, Huge).
    /// Defaults to Normal (1.0x). Enables accessibility for vision-impaired users
    /// and high-DPI displays.
    #[serde(default)]
    pub font_scale: FontScale,
    /// Enable persistent portal session tokens for libei/RemoteDesktop portal.
    /// When enabled, the daemon stores session restoration tokens to avoid
    /// showing consent dialogs on every reconnect. Tokens are stored in
    /// ~/.config/wayexpand/ with mode 0600 (user-only read/write).
    /// Defaults to true. Set to false to request fresh consent every time.
    /// Standalone processes use `$XDG_CONFIG_HOME/wayexpand` (or
    /// `$HOME/.config/wayexpand`); packaged systemd units provide the
    /// absolute `WAYEXPAND_PORTAL_TOKEN_PATH` override explicitly.
    #[serde(default = "default_libei_persistence")]
    pub libei_token_persistence: bool,
    /// Use an installed Fcitx5 direct-commit bridge when the libei route is
    /// active. The bridge is optional and disabled by default.
    #[serde(default)]
    pub fcitx5_direct_commit: bool,
    /// Environment variables snippets may read with `{{env:NAME}}`. Only
    /// names listed here are readable; any other name is a config error.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub template_env: Vec<String>,
    /// Allow `{{clipboard}}`. Off by default: the clipboard often holds
    /// passwords and other secrets. Organization policy can still block it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub allow_clipboard: bool,
    /// Record local usage statistics (snippet IDs, counts, and dates; never
    /// text) next to the configuration. On by default; nothing leaves the
    /// machine.
    #[serde(default = "default_usage_stats", skip_serializing_if = "is_true")]
    pub usage_stats: bool,
}

fn default_usage_stats() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_libei_persistence() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MatchMode {
    #[default]
    Immediate,
    WordBoundary,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FontScale {
    Small,
    #[default]
    Normal,
    Large,
    ExtraLarge,
    Huge,
}

impl FontScale {
    pub fn multiplier(&self) -> f32 {
        match self {
            FontScale::Small => 0.8,
            FontScale::Normal => 1.0,
            FontScale::Large => 1.2,
            FontScale::ExtraLarge => 1.5,
            FontScale::Huge => 2.0,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_buffer_chars: 128,
            undo_chord: None,
            font_scale: FontScale::Normal,
            libei_token_persistence: true,
            fcitx5_direct_commit: false,
            template_env: Vec::new(),
            allow_clipboard: false,
            usage_stats: true,
        }
    }
}

impl Settings {
    /// Check if settings match defaults (no custom configuration)
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// The explicitly selected source and matching strength of an app filter.
/// The serialized configuration remains a string for compatibility with the
/// existing TOML shape, but is parsed before matching so backends cannot
/// accidentally reinterpret a filter as a different kind of selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppFilter {
    AppIdExact(String),
    AppIdGlob(String),
    TitleContains(String),
}

impl AppFilter {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() || raw.contains('\0') {
            return None;
        }
        let (operator, value) = raw.split_once(':').unwrap_or(("app_id_exact", raw));
        let value = value.trim();
        if value.is_empty() || value.contains('\0') {
            return None;
        }
        let value = value.to_lowercase();
        match operator {
            "app_id_exact" => Some(Self::AppIdExact(value)),
            "app_id_glob" => Some(Self::AppIdGlob(value)),
            "title_contains" => Some(Self::TitleContains(value)),
            _ => None,
        }
    }

    pub fn is_weak(&self) -> bool {
        !matches!(self, Self::AppIdExact(_))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpansionConfig {
    /// Stable identity independent of the editable trigger. Older configs are
    /// migrated atomically on their first load and receive a persisted UUID.
    #[serde(default = "new_expansion_id")]
    pub id: String,
    pub trigger: String,
    /// Additional triggers for the same replacement, e.g. `:addr` with
    /// aliases `:address` and `:office`. Each alias follows the trigger's
    /// rules and match mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub replacement: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub category: String,
    /// App filters. Bare values are exact normalized desktop app IDs.
    /// Explicit operators are `app_id_exact:...`, `app_id_glob:...`, and
    /// `title_contains:...`; only the first is a security-strength match.
    /// Empty means unrestricted. If window tracking is unavailable, a
    /// non-empty filter fails closed rather than firing everywhere.
    #[serde(default)]
    pub app_filter: Vec<String>,
    #[serde(default)]
    pub match_mode: MatchMode,
    #[serde(default)]
    pub command: Option<CommandConfig>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// When the typed trigger is all-uppercase or capitalized, apply the
    /// same casing to the replacement before inserting it (e.g. typing
    /// `SIG` instead of `sig` yields an uppercased replacement). Mixed-case
    /// triggers are left unchanged because they do not communicate a reliable
    /// recasing rule. Off by default so existing configs keep behaving exactly
    /// as before.
    #[serde(default)]
    pub propagate_case: bool,
}

impl ExpansionConfig {
    /// Whether `trigger` is this snippet's trigger or one of its aliases,
    /// exactly as configured (no case or normalization variants).
    pub fn answers_to(&self, trigger: &str) -> bool {
        self.trigger == trigger || self.aliases.iter().any(|alias| alias == trigger)
    }

    /// Generate an RFC 4122 version-4 UUID for a new snippet.
    pub fn new_id() -> String {
        new_expansion_id()
    }

    /// The trigger strings this expansion actually inserts into the
    /// matcher's trie: just `trigger`, or (when `propagate_case` is
    /// enabled) `trigger` plus its uppercase and capitalized forms -- see
    /// `ExpansionEngine::new`, which builds the matcher from exactly this
    /// per expansion. `validate()` checks collisions across these effective
    /// triggers rather than the literal `trigger` field: two expansions
    /// whose configured triggers never collide as written can still
    /// collide once case variants are generated (`:sig` with
    /// `propagate_case` generates `:SIG`, which would otherwise silently
    /// shadow an unrelated, literally-configured `:SIG` expansion in the
    /// matcher with no validation error at all).
    /// Return the literal trigger plus the case variants generated when case
    /// propagation is enabled. Useful to avoid merge-time collisions using
    /// the same semantics as runtime matching and configuration validation.
    pub fn effective_triggers(&self) -> Vec<String> {
        // Insertion-ordered set: `variants` keeps the order callers rely on,
        // `seen` makes each duplicate check O(1) instead of a linear scan
        // over up to (aliases x case x normalization) variants.
        let mut variants = Vec::with_capacity(1 + self.aliases.len());
        let mut seen = std::collections::HashSet::with_capacity(1 + self.aliases.len());
        let mut push = |variants: &mut Vec<String>, candidate: String| {
            if seen.insert(candidate.clone()) {
                variants.push(candidate);
            }
        };
        for trigger in std::iter::once(&self.trigger).chain(&self.aliases) {
            push(&mut variants, trigger.clone());
        }
        if self.propagate_case {
            for index in 0..variants.len() {
                let uppercase = variants[index].to_uppercase();
                let capitalized = capitalize_first_letter(&variants[index]);
                push(&mut variants, uppercase);
                push(&mut variants, capitalized);
            }
        }
        // Canonically equivalent spellings must match too: a trigger saved
        // decomposed (`e` + U+0301, common in files from macOS) would
        // otherwise never fire for precomposed keyboard input, and vice
        // versa. Matching still consumes exactly the scalars that were
        // typed, so deletion counts stay correct.
        use unicode_normalization::UnicodeNormalization;
        for index in 0..variants.len() {
            let composed = variants[index].nfc().collect::<String>();
            let decomposed = variants[index].nfd().collect::<String>();
            push(&mut variants, composed);
            push(&mut variants, decomposed);
        }
        variants
    }
}

/// Capitalizes the first alphabetic character of `text`, leaving everything
/// else (including any non-alphabetic prefix, e.g. a `:` trigger sigil)
/// unchanged. Shared by `ExpansionConfig::effective_triggers` (to generate
/// the capitalized trigger variant) and the engine's replacement recasing
/// for `propagate_case` (to capitalize the *output* text the same way).
pub(crate) fn capitalize_first_letter(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalized = false;
    for character in text.chars() {
        if !capitalized && character.is_alphabetic() {
            result.extend(character.to_uppercase());
            capitalized = true;
        } else {
            result.push(character);
        }
    }
    result
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    /// Named policy-controlled action executed by the optional Action Broker.
    /// This is mutually exclusive with `program`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_command_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub cache_ms: u64,
    /// Environment policy for the child process. Minimal is the secure
    /// default; inherit must be explicitly requested for desktop commands.
    #[serde(default, skip_serializing_if = "CommandEnvironment::is_minimal")]
    pub environment: CommandEnvironment,
    /// Additional variables copied from the daemon environment in minimal
    /// mode. Values are never stored in the configuration file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_env: Vec<String>,
}

/// Validate the invariants shared by expansion and hotkey commands.
///
/// Context-specific checks, such as organization policy for executable paths,
/// remain at the owning configuration layer. Keeping the command shape and
/// resource limits here prevents the two command consumers from drifting.
pub fn validate_command_config(command: &CommandConfig) -> Result<(), &'static str> {
    let has_action = command.action.is_some();
    let has_program = !command.program.trim().is_empty();
    if has_action == has_program {
        return Err("command must specify exactly one of action or program");
    }
    if let Some(action) = &command.action {
        if action.trim().is_empty()
            || action.chars().count() > MAX_COMMAND_PROGRAM_CHARS
            || action.contains('\0')
        {
            return Err("action ID is empty, too long, or contains NUL");
        }
    }
    if command.program.chars().count() > MAX_COMMAND_PROGRAM_CHARS || command.program.contains('\0')
    {
        return Err("program is too long or contains NUL");
    }
    if command.args.len() > MAX_COMMAND_ARGS {
        return Err("too many arguments");
    }
    let mut argument_chars = 0usize;
    for argument in &command.args {
        if argument.chars().count() > MAX_COMMAND_ARG_CHARS || argument.contains('\0') {
            return Err("argument is too long or contains NUL");
        }
        argument_chars = argument_chars.saturating_add(argument.chars().count());
        if argument_chars > MAX_COMMAND_ARG_DATA_CHARS {
            return Err("argument data is too large");
        }
    }
    if !(1..=MAX_COMMAND_TIMEOUT_MS).contains(&command.timeout_ms) {
        return Err("timeout must be between 1 and 5000 milliseconds");
    }
    if command.cache_ms > MAX_COMMAND_CACHE_MS {
        return Err("cache must be between 0 and 60000 milliseconds");
    }
    if command.pass_env.len() > MAX_COMMAND_ENV_VARS
        || command.pass_env.iter().any(|name| {
            name.is_empty()
                || name.chars().count() > MAX_COMMAND_ENV_NAME_CHARS
                || name.contains('=')
                || name.contains('\0')
        })
    {
        return Err("pass_env contains an invalid or excessive environment name");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CommandEnvironment {
    #[default]
    Minimal,
    Inherit,
}

impl CommandEnvironment {
    fn is_minimal(&self) -> bool {
        matches!(self, Self::Minimal)
    }
}

fn default_command_timeout_ms() -> u64 {
    500
}

fn default_enabled() -> bool {
    true
}

/// Every enabled expansion's effective triggers as `(expansion index,
/// trigger)`, sorted by trigger and known to be collision-free. Validation
/// computes this anyway, so engine construction reuses it for the matcher.
pub(crate) type EffectiveTriggers = Vec<(usize, String)>;

impl Config {
    /// Validate and merge the primary configuration with every portable layer
    /// file next to it, so malformed files and cross-file collisions cannot be
    /// synchronized or reported valid unnoticed.
    pub fn validate_library_files(path: impl AsRef<Path>) -> Result<(), crate::FleetError> {
        let path = path.as_ref();
        let base = Self::load(path).map_err(crate::FleetError::Config)?;
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        crate::FleetConfig::validate_library(base, directory)
    }

    /// Validate each `snippets.d/*.toml` layer next to the configuration at
    /// `path`. Layers are loaded like the fleet loader does: symlinks are
    /// followed, and `Config::load` rejects non-regular or unsafe targets.
    pub fn validate_layer_files(path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let Some(directory) = path.as_ref().parent() else {
            return Ok(());
        };
        for file in Self::layer_files(directory)? {
            Self::load(file)?;
        }
        Ok(())
    }

    /// The sorted `snippets.d/*.toml` entries next to a configuration.
    pub fn layer_files(directory: &Path) -> Result<Vec<std::path::PathBuf>, ConfigError> {
        let snippets = directory.join("snippets.d");
        let entries = match fs::read_dir(&snippets) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: snippets.display().to_string(),
                    source,
                })
            }
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ConfigError::Read {
                path: snippets.display().to_string(),
                source,
            })?;
            let file = entry.path();
            if file
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                files.push(file);
            }
        }
        files.sort();
        Ok(files)
    }

    /// Static snippet replacements by trigger and alias, for
    /// `{{snippet:TRIGGER}}`. Command-backed and disabled snippets are not
    /// includable. Each snippet's replacement is stored once and shared by
    /// its trigger and every alias.
    pub fn includable_snippets(&self) -> std::sync::Arc<crate::SnippetLibrary> {
        let mut snippets = HashMap::new();
        for expansion in self.expansion.iter().filter(|expansion| {
            // Form snippets are not includable: their fields could
            // only be filled by the form of the snippet that owns them.
            expansion.enabled
                && expansion.command.is_none()
                && crate::form_fields(&expansion.replacement).is_ok_and(|fields| fields.is_empty())
        }) {
            let replacement: std::sync::Arc<str> = expansion.replacement.as_str().into();
            for trigger in std::iter::once(&expansion.trigger).chain(&expansion.aliases) {
                snippets.insert(trigger.clone(), std::sync::Arc::clone(&replacement));
            }
        }
        std::sync::Arc::new(snippets)
    }

    /// The context snippets render with: built-ins, allowlisted environment
    /// variables, includable snippets, and the clipboard reader when the user
    /// enabled `{{clipboard}}`. Organization policy (safe mode) can disable
    /// the environment and clipboard variables.
    pub fn template_context(&self, clipboard: Option<ClipboardReader>) -> TemplateContext {
        self.template_context_with_snippets(clipboard, self.includable_snippets())
    }

    /// [`Config::template_context`] with an already-built snippet library, so
    /// engine construction builds the library once for validation and use.
    pub(crate) fn template_context_with_snippets(
        &self,
        clipboard: Option<ClipboardReader>,
        snippets: std::sync::Arc<crate::SnippetLibrary>,
    ) -> TemplateContext {
        let enforcement = self.organization.effective_enforcement_policy();
        let env = if enforcement.disable_template_env {
            std::collections::BTreeMap::new()
        } else {
            self.settings
                .template_env
                .iter()
                .map(|name| (name.clone(), std::env::var(name).unwrap_or_default()))
                .collect()
        };
        TemplateContext {
            env: std::sync::Arc::new(env),
            snippets,
            clipboard: clipboard
                .filter(|_| self.settings.allow_clipboard && !enforcement.disable_clipboard),
            ..TemplateContext::system()
        }
    }

    /// The context used to validate templates: the same variables are
    /// allowed as at runtime, but nothing is read.
    fn validation_template_context(
        &self,
        snippets: std::sync::Arc<crate::SnippetLibrary>,
    ) -> TemplateContext {
        TemplateContext {
            env: std::sync::Arc::new(
                self.settings
                    .template_env
                    .iter()
                    .map(|name| (name.clone(), String::new()))
                    .collect(),
            ),
            snippets,
            clipboard: self
                .settings
                .allow_clipboard
                .then(|| ClipboardReader(std::sync::Arc::new(|| Some(String::new())))),
            validating: true,
            ..TemplateContext::default()
        }
    }

    /// Create the empty per-user configuration when it does not exist, then
    /// load it through the same secure path used for existing files.
    ///
    /// This is intentionally an explicit onboarding operation. Read-only
    /// commands such as `validate` and `doctor` continue to report a missing
    /// configuration instead of mutating the user's filesystem.
    pub fn ensure_user_config(path: impl AsRef<Path>) -> Result<LoadedConfig, ConfigError> {
        let path = path.as_ref();
        match Self::load_versioned(path) {
            Ok(loaded) => Ok(loaded),
            Err(ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    let parent_exists = parent.exists();
                    fs::create_dir_all(parent).map_err(|source| ConfigError::Read {
                        path: parent.display().to_string(),
                        source,
                    })?;
                    if !parent_exists {
                        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(
                            |source| ConfigError::Read {
                                path: parent.display().to_string(),
                                source,
                            },
                        )?;
                    }
                }
                let config = Self {
                    expansion: Vec::new(),
                    hotkey: Vec::new(),
                    settings: Settings::default(),
                    organization: OrganizationPolicy::default(),
                };
                config.save_atomic(path)?;
                Self::load_versioned(path)
            }
            Err(error) => Err(error),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_versioned(path).map(|loaded| loaded.config)
    }

    /// Load, validate, and retain the exact source document from one secure
    /// descriptor read. Legacy snippet IDs are migrated atomically before
    /// returning. The migration is serialized and rechecks the file under the
    /// write lock, so concurrent readers converge on one ID set. Editors
    /// should keep the revision and use it for conditional saves rather than
    /// re-reading the file independently.
    pub fn load_versioned(path: impl AsRef<Path>) -> Result<LoadedConfig, ConfigError> {
        let path = path.as_ref();
        let loaded = Self::load_versioned_once(path)?;
        if !source_has_legacy_expansion_ids(&loaded.source) {
            return Ok(loaded);
        }

        let resolved = resolve_config_target(path)?;
        validate_parent_directories(&resolved)?;
        let _lock = ConfigWriteLock::acquire(&resolved)?;
        let current = Self::load_versioned_once(&resolved)?;
        if source_has_legacy_expansion_ids(&current.source) {
            current.config.validate()?;
            let serialized = toml::to_string_pretty(&current.config)?;
            Self::save_atomic_serialized(&resolved, serialized.as_bytes())?;
            Self::load_versioned_once(&resolved)
        } else {
            Ok(current)
        }
    }

    fn load_versioned_once(path: impl AsRef<Path>) -> Result<LoadedConfig, ConfigError> {
        let path = path.as_ref();
        // Resolve symlinks before validating ancestors and opening the file.
        // Validating the link's parent alone would allow a link swap to point
        // at a file below an untrusted directory. Opening the resolved path
        // also makes the validated location the one read by this call.
        let resolved_path = fs::canonicalize(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        validate_parent_directories(&resolved_path)?;
        // Open once and validate the resulting descriptor. O_NONBLOCK keeps a
        // FIFO or device node from blocking the service before we can reject
        // it, and descriptor metadata removes the path check/open race.
        let descriptor = rustix::fs::open(
            &resolved_path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )
        .map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source: source.into(),
        })?;
        let file = fs::File::from(descriptor);
        let metadata = file.metadata().map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(ConfigError::NotRegular {
                path: path.display().to_string(),
            });
        }
        let uid = metadata.uid();
        let current_uid = rustix::process::geteuid().as_raw();
        if uid != current_uid && uid != 0 {
            return Err(ConfigError::InsecureOwner {
                path: path.display().to_string(),
                uid,
            });
        }
        if uid == 0 {
            validate_root_managed_parent_chain(&resolved_path)?;
        }
        let mode = metadata.permissions().mode() & 0o777;
        // Personal configuration may contain private snippets, addresses, and
        // command arguments, so require a private 0600 confidentiality
        // boundary. Root-owned managed configuration is an administrator-
        // controlled integrity boundary and may remain readable (for example
        // 0644), provided no non-root user can modify it.
        let permissions_insecure = if uid == current_uid {
            mode != 0o600
        } else {
            mode & 0o022 != 0
        };
        if permissions_insecure {
            return Err(ConfigError::InsecurePermissions {
                path: path.display().to_string(),
                mode,
            });
        }
        if metadata.len() > MAX_CONFIG_BYTES as u64 {
            return Err(ConfigError::ConfigTooLarge {
                length: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| ConfigError::Read {
                path: path.display().to_string(),
                source,
            })?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::ConfigTooLarge {
                length: bytes.len(),
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let text = String::from_utf8(bytes).map_err(|source| ConfigError::InvalidUtf8 {
            path: path.display().to_string(),
            source,
        })?;
        let config = Self::parse(&text)?;
        let source: Arc<str> = Arc::from(text);
        Ok(LoadedConfig {
            config,
            revision: ConfigRevision(source.clone()),
            source,
        })
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::ConfigTooLarge {
                length: text.len(),
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Atomically replace a trusted configuration file. The full document is
    /// serialized with canonical TOML formatting, so comments and manual
    /// formatting are not preserved. A missing target is created with private
    /// permissions, which lets settings frontends initialize a first-run
    /// library without weakening the same parent and ownership checks used for
    /// existing files.
    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        self.validate()?;
        let serialized = toml::to_string_pretty(self)?;
        Self::save_text_unconditionally(path, &serialized)
    }

    /// Atomically replace a trusted configuration file with TOML supplied by
    /// a format-preserving editor. Parse and validate it before touching the
    /// target so callers cannot bypass the configuration safety checks.
    pub fn save_atomic_text(path: impl AsRef<Path>, text: &str) -> Result<(), ConfigError> {
        Self::parse(text)?;
        Self::save_text_unconditionally(path, text)
    }

    fn save_text_unconditionally(path: impl AsRef<Path>, text: &str) -> Result<(), ConfigError> {
        let resolved = resolve_config_target(path.as_ref())?;
        validate_parent_directories(&resolved)?;
        let _lock = ConfigWriteLock::acquire(&resolved)?;
        Self::save_atomic_serialized(&resolved, text.as_bytes())
    }

    /// Save only if the exact validated source snapshot is still current.
    /// A stable per-path advisory lock serializes cooperating WayExpand
    /// writers; the revision comparison also detects changes made by editors
    /// and tools that do not participate in that lock protocol.
    pub fn save_atomic_if_revision_matches(
        &self,
        path: impl AsRef<Path>,
        expected: &ConfigRevision,
    ) -> Result<ConfigRevision, ConfigError> {
        self.validate()?;
        let serialized = toml::to_string_pretty(self)?;
        Self::save_text_if_revision_matches(path, &serialized, expected)
    }

    /// Format-preserving counterpart to `save_atomic_if_revision_matches`.
    pub fn save_atomic_text_if_revision_matches(
        path: impl AsRef<Path>,
        text: &str,
        expected: &ConfigRevision,
    ) -> Result<ConfigRevision, ConfigError> {
        Self::parse(text)?;
        Self::save_text_if_revision_matches(path, text, expected)
    }

    fn save_text_if_revision_matches(
        path: impl AsRef<Path>,
        text: &str,
        expected: &ConfigRevision,
    ) -> Result<ConfigRevision, ConfigError> {
        let resolved = resolve_config_target(path.as_ref())?;
        validate_parent_directories(&resolved)?;
        let _lock = ConfigWriteLock::acquire(&resolved)?;
        let current = Self::load_versioned(&resolved)?;
        if current.revision != *expected {
            return Err(ConfigError::RevisionConflict);
        }
        Self::save_atomic_serialized(&resolved, text.as_bytes())?;
        Ok(ConfigRevision(Arc::from(text)))
    }

    #[cfg(not(target_os = "linux"))]
    fn save_atomic_serialized(
        path: impl AsRef<Path>,
        serialized: &[u8],
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let resolved = resolve_config_target(path)?;
        validate_parent_directories(&resolved)?;
        Self::validate_save_target_owner(&resolved)?;
        let parent = resolved.parent().unwrap_or_else(|| Path::new("."));
        let file_name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("expansions.toml");
        let mut last_error = None;
        for attempt in 0..16 {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let temp = parent.join(format!(
                ".{file_name}.tmp.{}.{}.{}",
                std::process::id(),
                nonce,
                attempt
            ));
            let mut file = match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
            {
                Ok(file) => file,
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(ConfigError::Read {
                        path: temp.display().to_string(),
                        source,
                    });
                    continue;
                }
                Err(source) => {
                    return Err(ConfigError::Read {
                        path: temp.display().to_string(),
                        source,
                    });
                }
            };
            let result = (|| -> Result<(), ConfigError> {
                file.write_all(serialized)
                    .map_err(|source| ConfigError::Read {
                        path: temp.display().to_string(),
                        source,
                    })?;
                file.sync_all().map_err(|source| ConfigError::Read {
                    path: temp.display().to_string(),
                    source,
                })?;
                // Recheck immediately before replacement to avoid downgrading
                // a target that became root-owned while the temporary file was
                // being written. Linux uses the descriptor-relative variant
                // below; this path is the portable fallback.
                Self::validate_save_target_owner(&resolved)?;
                fs::rename(&temp, &resolved).map_err(|source| ConfigError::Read {
                    path: resolved.display().to_string(),
                    source,
                })?;
                fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|source| ConfigError::Read {
                        path: parent.display().to_string(),
                        source,
                    })?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            return result;
        }
        Err(last_error.unwrap_or_else(|| ConfigError::Read {
            path: parent.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique configuration temporary file",
            ),
        }))
    }

    #[cfg(target_os = "linux")]
    fn save_atomic_serialized(
        path: impl AsRef<Path>,
        serialized: &[u8],
    ) -> Result<(), ConfigError> {
        let resolved = resolve_config_target(path.as_ref())?;
        validate_parent_directories(&resolved)?;
        Self::validate_save_target_owner(&resolved)?;
        let file_name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("expansions.toml");
        save_atomic_serialized_relative(&resolved, file_name, serialized)
    }

    fn validate_save_target_owner(path: &Path) -> Result<(), ConfigError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.display().to_string(),
                    source,
                });
            }
        };
        if !metadata.file_type().is_file() {
            return Err(ConfigError::NotRegular {
                path: path.display().to_string(),
            });
        }
        let current_uid = rustix::process::geteuid().as_raw();
        if root_owned_target_requires_admin(metadata.uid(), current_uid) {
            return Err(ConfigError::RootOwnedWriteRequiresAdmin {
                path: path.display().to_string(),
            });
        }
        if metadata.uid() != current_uid {
            return Err(ConfigError::InsecureOwner {
                path: path.display().to_string(),
                uid: metadata.uid(),
            });
        }
        Ok(())
    }
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

mod error;
mod organization;
mod storage;
mod validation;

pub use error::ConfigError;
pub use organization::OrganizationPolicy;
use storage::{
    resolve_config_target, root_owned_target_requires_admin, save_atomic_serialized_relative,
    validate_parent_directories, validate_root_managed_parent_chain, ConfigWriteLock,
};

#[cfg(test)]
mod tests;
