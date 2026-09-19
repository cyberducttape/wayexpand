use crate::{render_template_with_cursor, KeyChord, TemplateContext, TemplateError};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use thiserror::Error;

const MAX_TRIGGER_CHARS: usize = 128;
const MAX_REPLACEMENT_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 512;
const MAX_TAGS: usize = 32;
const MAX_TAG_CHARS: usize = 64;
const MAX_CATEGORY_CHARS: usize = 64;
const MAX_APP_FILTERS: usize = 32;
const MAX_APP_FILTER_CHARS: usize = 256;
const MAX_COMMAND_ARGS: usize = 32;
const MAX_COMMAND_PROGRAM_CHARS: usize = 256;
const MAX_COMMAND_ARG_CHARS: usize = 1024;
const MAX_COMMAND_ARG_DATA_CHARS: usize = 16 * 1024;
const MAX_COMMAND_ENV_VARS: usize = 32;
const MAX_COMMAND_ENV_NAME_CHARS: usize = 256;
const MAX_COMMAND_TIMEOUT_MS: u64 = 5_000;
const MAX_COMMAND_CACHE_MS: u64 = 60_000;
const MAX_EXPANSIONS: usize = 10_000;
const MAX_HOTKEYS: usize = 1_024;
const MAX_HOTKEY_DESCRIPTION_CHARS: usize = 256;
pub(crate) const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TOTAL_TRIGGER_CHARS: usize = 256 * 1024;

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

/// A keyboard chord which invokes a bounded direct program action.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    #[serde(default = "default_libei_persistence")]
    pub libei_token_persistence: bool,
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
        }
    }
}

impl Settings {
    /// Check if settings match defaults (no custom configuration)
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Organization-managed policy for compliance and security.
///
/// Root-owned policies enforce constraints on user expansions, preventing
/// accidental or malicious use in sensitive contexts.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct OrganizationPolicy {
    /// Enable strict policy enforcement. When true, any policy violation
    /// is logged as an error and prevents the expansion from executing.
    /// When false, violations are logged as warnings but expansions proceed.
    /// This allows organizations to audit policy behavior before full enforcement.
    pub safe_mode: bool,

    /// Disable command execution entirely. Overrides individual expansion
    /// command settings. Useful for locked-down environments.
    pub disable_commands: bool,

    /// Disable hotkey execution. Hotkeys still parse but refuse to run.
    pub disable_hotkeys: bool,

    /// Disable title-based app filtering. Expansions with app_filter still
    /// match, but the filter is ignored (all apps match).
    pub disable_title_matching: bool,

    /// Maximum replacement size in bytes. Replacements larger than this
    /// are rejected. Prevents DoS via huge expansions. 0 = unlimited.
    pub max_replacement_size: usize,

    /// Allowed output backends. If non-empty, only these backends are allowed.
    /// Examples: "libei", "input-method", "wlroots", "none"
    pub allowed_backends: Vec<String>,

    /// Allowed curated packs. If non-empty, only these packs are allowed
    /// in ~/.local/share/wayexpand/packs/. Pack names must match directory names.
    pub allowed_packs: Vec<String>,

    /// Policy violation audit log. When violations occur, they're logged
    /// to journald with this prefix for easy filtering.
    pub audit_prefix: String,
}

impl Default for OrganizationPolicy {
    fn default() -> Self {
        Self {
            safe_mode: false,
            disable_commands: false,
            disable_hotkeys: false,
            disable_title_matching: false,
            max_replacement_size: 0,
            allowed_backends: Vec::new(),
            allowed_packs: Vec::new(),
            audit_prefix: "wayexpand-policy".to_string(),
        }
    }
}

impl OrganizationPolicy {
    /// Check if any policies are active
    pub fn is_active(&self) -> bool {
        self.safe_mode
            || self.disable_commands
            || self.disable_hotkeys
            || self.disable_title_matching
            || self.max_replacement_size > 0
            || !self.allowed_backends.is_empty()
            || !self.allowed_packs.is_empty()
    }

    /// Check if policy allows a backend
    pub fn backend_allowed(&self, backend: &str) -> bool {
        if self.allowed_backends.is_empty() {
            true
        } else {
            self.allowed_backends.iter().any(|b| b == backend)
        }
    }

    /// Check if policy allows a pack
    pub fn pack_allowed(&self, pack_name: &str) -> bool {
        if self.allowed_packs.is_empty() {
            true
        } else {
            self.allowed_packs.iter().any(|p| p == pack_name)
        }
    }

    /// Check if replacement size is allowed
    pub fn replacement_size_allowed(&self, size: usize) -> bool {
        if self.max_replacement_size == 0 {
            true
        } else {
            size <= self.max_replacement_size
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionConfig {
    pub trigger: String,
    pub replacement: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub category: String,
    /// Case-insensitive substrings matched against the focused window's
    /// app id or title. Empty means unrestricted. If window tracking is
    /// unavailable on the running compositor, a non-empty filter fails
    /// closed (the expansion never matches) rather than firing everywhere.
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
    /// `SIG` instead of `sig` yields an uppercased replacement). Off by
    /// default so existing configs keep behaving exactly as before.
    #[serde(default)]
    pub propagate_case: bool,
}

impl ExpansionConfig {
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
    pub(crate) fn effective_triggers(&self) -> Vec<String> {
        let mut variants = vec![self.trigger.clone()];
        if self.propagate_case {
            for variant in [
                self.trigger.to_uppercase(),
                capitalize_first_letter(&self.trigger),
            ] {
                if !variants.contains(&variant) {
                    variants.push(variant);
                }
            }
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

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("configuration {path} is not a regular file")]
    NotRegular { path: String },
    #[error("configuration {path} is writable by group or other users (mode {mode:04o})")]
    InsecurePermissions { path: String, mode: u32 },
    #[error("configuration {path} is owned by uid {uid}; expected the current user or root")]
    InsecureOwner { path: String, uid: u32 },
    #[error(
        "configuration parent {path} is writable by group or other users without sticky protection (mode {mode:04o})"
    )]
    InsecureParent { path: String, mode: u32 },
    #[error(
        "configuration parent {path} is owned by uid {uid}; expected the current user or root"
    )]
    InsecureParentOwner { path: String, uid: u32 },
    #[error("configuration {path} is not valid UTF-8: {source}")]
    InvalidUtf8 {
        path: String,
        source: std::string::FromUtf8Error,
    },
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("expansion {index} has an empty trigger")]
    EmptyTrigger { index: usize },
    #[error("expansion {index} {field} contains a NUL character")]
    NulCharacter { index: usize, field: &'static str },
    #[error("expansion {index} trigger is too long ({length} characters; maximum is {maximum})")]
    TriggerTooLong {
        index: usize,
        length: usize,
        maximum: usize,
    },
    #[error("expansion {index} replacement is too large ({length} bytes; maximum is {maximum})")]
    ReplacementTooLarge {
        index: usize,
        length: usize,
        maximum: usize,
    },
    #[error("expansion {index} description is too long (maximum is {maximum} characters)")]
    DescriptionTooLong { index: usize, maximum: usize },
    #[error("expansion {index} has invalid tags")]
    InvalidTags { index: usize },
    #[error("expansion {index} has an invalid app filter")]
    InvalidAppFilter { index: usize },
    #[error("expansion {index} has an invalid category")]
    InvalidCategory { index: usize },
    #[error("expansion {index} has an invalid command: {reason}")]
    InvalidCommand { index: usize, reason: &'static str },
    #[error("duplicate trigger {trigger:?} in expansions {first} and {second}")]
    DuplicateTrigger {
        trigger: String,
        first: usize,
        second: usize,
    },
    #[error("max_buffer_chars must be between 1 and 4096")]
    InvalidBufferLimit,
    #[error("settings.undo_chord is empty, ambiguous, or contains an unknown modifier")]
    InvalidUndoChord,
    #[error("configuration contains {count} expansions; maximum is {maximum}")]
    TooManyExpansions { count: usize, maximum: usize },
    #[error("configuration contains {count} hotkeys; maximum is {maximum}")]
    TooManyHotkeys { count: usize, maximum: usize },
    #[error("hotkey {index} is invalid: {reason}")]
    InvalidHotkey { index: usize, reason: &'static str },
    #[error("duplicate hotkey {chord:?} in entries {first} and {second}")]
    DuplicateHotkey {
        chord: String,
        first: usize,
        second: usize,
    },
    #[error("enabled triggers contain {length} characters; maximum is {maximum}")]
    TriggerDataTooLarge { length: usize, maximum: usize },
    #[error("configuration is too large ({length} bytes; maximum is {maximum})")]
    ConfigTooLarge { length: usize, maximum: usize },
    #[error("could not serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("expansion {index} has an invalid replacement template: {source}")]
    InvalidTemplate { index: usize, source: TemplateError },
    #[error("organization policy configuration error: {0}")]
    InvalidPolicyConfig(String),
}

impl ConfigError {
    /// Return operator-useful diagnostics without echoing configuration text.
    /// The `Display` implementation remains detailed for library callers, but
    /// user-facing health checks should use this boundary-safe form.
    pub fn safe_summary(&self) -> String {
        match self {
            Self::Read { source, .. } => format!("read failed ({})", source.kind()),
            Self::NotRegular { .. } => "path is not a regular file".into(),
            Self::InsecurePermissions { mode, .. } => {
                format!("file permissions are insecure (mode {mode:04o})")
            }
            Self::InsecureOwner { uid, .. } => format!("file owner is not trusted (uid {uid})"),
            Self::InsecureParent { mode, .. } => {
                format!("parent directory is insecure (mode {mode:04o})")
            }
            Self::InsecureParentOwner { uid, .. } => {
                format!("parent directory owner is not trusted (uid {uid})")
            }
            Self::InvalidUtf8 { source, .. } => {
                format!(
                    "file is not valid UTF-8 (invalid at byte {})",
                    source.utf8_error().valid_up_to()
                )
            }
            Self::Parse(_) => "invalid TOML".into(),
            Self::EmptyTrigger { index } => format!("expansion {index} has an empty trigger"),
            Self::NulCharacter { index, field } => {
                format!("expansion {index} {field} contains a NUL character")
            }
            Self::TriggerTooLong {
                index,
                length,
                maximum,
            } => format!("expansion {index} trigger is too long ({length}; maximum {maximum})"),
            Self::ReplacementTooLarge {
                index,
                length,
                maximum,
            } => {
                format!("expansion {index} replacement is too large ({length}; maximum {maximum})")
            }
            Self::DescriptionTooLong { index, maximum } => {
                format!("expansion {index} description is too long (maximum {maximum})")
            }
            Self::InvalidTags { index } => format!("expansion {index} has invalid tags"),
            Self::InvalidAppFilter { index } => {
                format!("expansion {index} has an invalid app filter")
            }
            Self::InvalidCategory { index } => {
                format!("expansion {index} has an invalid category")
            }
            Self::InvalidCommand { index, reason } => {
                format!("expansion {index} has an invalid command ({reason})")
            }
            Self::DuplicateTrigger { first, second, .. } => {
                format!("duplicate trigger in expansions {first} and {second}")
            }
            Self::InvalidBufferLimit => "max_buffer_chars is outside the allowed range".into(),
            Self::InvalidUndoChord => "settings.undo_chord is invalid".into(),
            Self::TooManyExpansions { count, maximum } => {
                format!("too many expansions ({count}; maximum {maximum})")
            }
            Self::TooManyHotkeys { count, maximum } => {
                format!("too many hotkeys ({count}; maximum {maximum})")
            }
            Self::InvalidHotkey { index, reason } => {
                format!("hotkey {index} is invalid ({reason})")
            }
            Self::DuplicateHotkey { first, second, .. } => {
                format!("duplicate hotkey in entries {first} and {second}")
            }
            Self::TriggerDataTooLarge { length, maximum } => {
                format!("enabled trigger data is too large ({length}; maximum {maximum})")
            }
            Self::ConfigTooLarge { length, maximum } => {
                format!("configuration is too large ({length} bytes; maximum {maximum})")
            }
            Self::Serialize(_) => "configuration could not be serialized".into(),
            Self::InvalidTemplate { index, source } => {
                format!("expansion {index} has an invalid replacement template ({source})")
            }
            Self::InvalidPolicyConfig(msg) => {
                format!("organization policy configuration error: {msg}")
            }
        }
    }
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
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
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o022 != 0 {
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
        Self::parse(&text)
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

    /// Atomically replace a trusted configuration file. A missing target is
    /// created with private permissions, which lets settings frontends
    /// initialize a first-run library without weakening the same parent and
    /// ownership checks used for existing files.
    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        self.validate()?;
        let path = path.as_ref();
        let resolved = match fs::canonicalize(path) {
            Ok(resolved) => resolved,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let parent = fs::canonicalize(parent).map_err(|source| ConfigError::Read {
                    path: parent.display().to_string(),
                    source,
                })?;
                let file_name = path.file_name().ok_or_else(|| ConfigError::Read {
                    path: path.display().to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "configuration path has no file name",
                    ),
                })?;
                parent.join(file_name)
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        validate_parent_directories(&resolved)?;
        let parent = resolved.parent().unwrap_or_else(|| Path::new("."));
        let file_name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("expansions.toml");
        let temp = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));
        let serialized = toml::to_string_pretty(self)?;
        let result = (|| -> Result<(), ConfigError> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
                .map_err(|source| ConfigError::Read {
                    path: temp.display().to_string(),
                    source,
                })?;
            file.write_all(serialized.as_bytes())
                .map_err(|source| ConfigError::Read {
                    path: temp.display().to_string(),
                    source,
                })?;
            file.sync_all().map_err(|source| ConfigError::Read {
                path: temp.display().to_string(),
                source,
            })?;
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
        result
    }

    /// Validate a configuration assembled through the public Rust API.
    /// Parsing is not the only way callers can construct `Config`, so engine
    /// construction and other consumers can enforce the same limits here.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.expansion.len() > MAX_EXPANSIONS {
            return Err(ConfigError::TooManyExpansions {
                count: self.expansion.len(),
                maximum: MAX_EXPANSIONS,
            });
        }
        if !(1..=4096).contains(&self.settings.max_buffer_chars) {
            return Err(ConfigError::InvalidBufferLimit);
        }
        if let Some(undo_chord) = &self.settings.undo_chord {
            KeyChord::parse(undo_chord).map_err(|_| ConfigError::InvalidUndoChord)?;
        }
        if self.hotkey.len() > MAX_HOTKEYS {
            return Err(ConfigError::TooManyHotkeys {
                count: self.hotkey.len(),
                maximum: MAX_HOTKEYS,
            });
        }
        let mut hotkeys = Vec::new();
        for (index, binding) in self.hotkey.iter().enumerate() {
            let chord =
                KeyChord::parse(&binding.chord).map_err(|_| ConfigError::InvalidHotkey {
                    index,
                    reason: "chord is empty, ambiguous, or contains an unknown modifier",
                })?;
            if binding.description.chars().count() > MAX_HOTKEY_DESCRIPTION_CHARS
                || binding.description.contains('\0')
            {
                return Err(ConfigError::InvalidHotkey {
                    index,
                    reason: "description is too long or contains NUL",
                });
            }
            if binding.command.program.trim().is_empty()
                || binding.command.program.chars().count() > MAX_COMMAND_PROGRAM_CHARS
                || binding.command.program.contains('\0')
                || binding.command.args.len() > MAX_COMMAND_ARGS
                || !(1..=MAX_COMMAND_TIMEOUT_MS).contains(&binding.command.timeout_ms)
                || binding.command.cache_ms > MAX_COMMAND_CACHE_MS
                || binding.command.pass_env.len() > MAX_COMMAND_ENV_VARS
                || binding.command.pass_env.iter().any(|name| {
                    name.is_empty()
                        || name.chars().count() > MAX_COMMAND_ENV_NAME_CHARS
                        || name.contains('=')
                        || name.contains('\0')
                })
            {
                return Err(ConfigError::InvalidHotkey {
                    index,
                    reason: "command limits are invalid",
                });
            }
            let mut argument_chars = 0usize;
            for argument in &binding.command.args {
                if argument.chars().count() > MAX_COMMAND_ARG_CHARS || argument.contains('\0') {
                    return Err(ConfigError::InvalidHotkey {
                        index,
                        reason: "command argument is too long or contains NUL",
                    });
                }
                argument_chars = argument_chars.saturating_add(argument.chars().count());
                if argument_chars > MAX_COMMAND_ARG_DATA_CHARS {
                    return Err(ConfigError::InvalidHotkey {
                        index,
                        reason: "command argument data is too large",
                    });
                }
            }
            if binding.enabled {
                hotkeys.push((index, chord.to_string()));
            }
        }
        hotkeys.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        for pair in hotkeys.windows(2) {
            if pair[0].1 == pair[1].1 {
                return Err(ConfigError::DuplicateHotkey {
                    chord: pair[0].1.clone(),
                    first: pair[0].0,
                    second: pair[1].0,
                });
            }
        }
        let mut total_trigger_chars = 0usize;
        for (index, expansion) in self.expansion.iter().enumerate() {
            if expansion.trigger.is_empty() {
                return Err(ConfigError::EmptyTrigger { index });
            }
            if expansion.trigger.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "trigger",
                });
            }
            if expansion.replacement.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "replacement",
                });
            }
            let trigger_length = expansion.trigger.chars().count();
            if trigger_length > MAX_TRIGGER_CHARS {
                return Err(ConfigError::TriggerTooLong {
                    index,
                    length: trigger_length,
                    maximum: MAX_TRIGGER_CHARS,
                });
            }
            if expansion.replacement.len() > MAX_REPLACEMENT_BYTES {
                return Err(ConfigError::ReplacementTooLarge {
                    index,
                    length: expansion.replacement.len(),
                    maximum: MAX_REPLACEMENT_BYTES,
                });
            }
            if expansion.description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(ConfigError::DescriptionTooLong {
                    index,
                    maximum: MAX_DESCRIPTION_CHARS,
                });
            }
            if expansion.description.contains('\0') {
                return Err(ConfigError::NulCharacter {
                    index,
                    field: "description",
                });
            }
            if expansion.tags.len() > MAX_TAGS
                || expansion
                    .tags
                    .iter()
                    .any(|tag| tag.chars().count() > MAX_TAG_CHARS || tag.contains('\0'))
            {
                return Err(ConfigError::InvalidTags { index });
            }
            if expansion.app_filter.len() > MAX_APP_FILTERS
                || expansion.app_filter.iter().any(|filter| {
                    filter.is_empty()
                        || filter.chars().count() > MAX_APP_FILTER_CHARS
                        || filter.contains('\0')
                })
            {
                return Err(ConfigError::InvalidAppFilter { index });
            }
            if expansion.category.chars().count() > MAX_CATEGORY_CHARS
                || expansion.category.contains('\0')
            {
                return Err(ConfigError::InvalidCategory { index });
            }
            if let Some(command) = &expansion.command {
                if command.program.trim().is_empty() {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "program is empty",
                    });
                }
                if command.program.chars().count() > MAX_COMMAND_PROGRAM_CHARS
                    || command.program.contains('\0')
                {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "program is too long or contains NUL",
                    });
                }
                if command.args.len() > MAX_COMMAND_ARGS {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "too many arguments",
                    });
                }
                let mut argument_chars = 0usize;
                for argument in &command.args {
                    if argument.chars().count() > MAX_COMMAND_ARG_CHARS || argument.contains('\0') {
                        return Err(ConfigError::InvalidCommand {
                            index,
                            reason: "argument is too long or contains NUL",
                        });
                    }
                    argument_chars = argument_chars.saturating_add(argument.chars().count());
                    if argument_chars > MAX_COMMAND_ARG_DATA_CHARS {
                        return Err(ConfigError::InvalidCommand {
                            index,
                            reason: "argument data is too large",
                        });
                    }
                }
                if !(1..=MAX_COMMAND_TIMEOUT_MS).contains(&command.timeout_ms) {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "timeout must be between 1 and 5000 milliseconds",
                    });
                }
                if command.cache_ms > MAX_COMMAND_CACHE_MS {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "cache must be between 0 and 60000 milliseconds",
                    });
                }
                if command.pass_env.len() > MAX_COMMAND_ENV_VARS
                    || command.pass_env.iter().any(|name| {
                        name.is_empty()
                            || name.chars().count() > MAX_COMMAND_ENV_NAME_CHARS
                            || name.contains('=')
                            || name.contains('\0')
                    })
                {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "pass_env contains an invalid or excessive environment name",
                    });
                }
            } else if let Err(source) =
                render_template_with_cursor(&expansion.replacement, &TemplateContext::default())
            {
                return Err(ConfigError::InvalidTemplate { index, source });
            }
            if expansion.enabled {
                total_trigger_chars = total_trigger_chars.saturating_add(trigger_length);
                if total_trigger_chars > MAX_TOTAL_TRIGGER_CHARS {
                    return Err(ConfigError::TriggerDataTooLarge {
                        length: total_trigger_chars,
                        maximum: MAX_TOTAL_TRIGGER_CHARS,
                    });
                }
            }
        }

        // Sorting makes duplicate validation O(n log n) instead of comparing
        // every enabled expansion with every other expansion. Prefixes are
        // intentionally allowed; the matcher selects the longest suffix.
        //
        // This checks *effective* triggers (literal trigger, plus any
        // propagate_case-generated variants), not just the literal
        // `trigger` field: the matcher is built from effective triggers
        // (see `ExpansionEngine::new`), so two expansions with distinct
        // configured triggers can still collide once case variants are
        // generated -- and without this, that collision would silently
        // make one expansion unreachable instead of failing validation.
        let mut enabled: Vec<(usize, String)> = self
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.enabled)
            .flat_map(|(index, entry)| {
                entry
                    .effective_triggers()
                    .into_iter()
                    .map(move |trigger| (index, trigger))
            })
            .collect();
        enabled.sort_unstable_by(|left, right| {
            left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
        });
        for pair in enabled.windows(2) {
            let (first_index, first) = &pair[0];
            let (second_index, second) = &pair[1];
            if first == second {
                return Err(ConfigError::DuplicateTrigger {
                    trigger: first.clone(),
                    first: *first_index,
                    second: *second_index,
                });
            }
        }

        // Validate organization policies
        if self.organization.max_replacement_size > 0
            && self.organization.max_replacement_size < 256
        {
            return Err(ConfigError::InvalidPolicyConfig(
                "max_replacement_size must be 0 (unlimited) or at least 256 bytes".to_string(),
            ));
        }

        // Validate that safe_mode doesn't ban all backends
        if self.organization.safe_mode
            && !self.organization.allowed_backends.is_empty()
            && self
                .organization
                .allowed_backends
                .iter()
                .all(|b| b == "none")
        {
            return Err(ConfigError::InvalidPolicyConfig(
                "safe_mode with allowed_backends=['none'] would prevent all expansions".to_string(),
            ));
        }

        Ok(())
    }
}

fn validate_parent_directories(path: &Path) -> Result<(), ConfigError> {
    let mut current = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let current_uid = rustix::process::geteuid().as_raw();
    loop {
        let metadata = fs::metadata(current).map_err(|source| ConfigError::Read {
            path: current.display().to_string(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(ConfigError::Read {
                path: current.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "configuration parent is not a directory",
                ),
            });
        }
        let mode = metadata.permissions().mode() & 0o7777;
        let sticky = mode & 0o1000 != 0;
        let uid = metadata.uid();
        if uid != current_uid && uid != 0 {
            return Err(ConfigError::InsecureParentOwner {
                path: current.display().to_string(),
                uid,
            });
        }
        if mode & 0o022 != 0 && !sticky {
            return Err(ConfigError::InsecureParent {
                path: current.display().to_string(),
                mode,
            });
        }
        // NOTE: We intentionally do NOT stop at the first user-owned directory.
        // While a secure user-owned directory itself cannot be swapped
        // (it requires write access to its parent), a world-writable,
        // non-sticky parent directory can still allow another user to
        // rename/replace that directory entry.
        //
        // Example: /shared is world-writable and non-sticky, /shared/stephan
        // is 0700 and owned by stephan. A different user CAN rename
        // /shared/stephan to /shared/stephan.bak and create a new
        // /shared/stephan pointing to attacker-controlled config.
        //
        // Therefore, validate all ancestors up to "/" (except in a systemd
        // private namespace, where the overflow uid 65534 is remapped and
        // would cause false rejections). Until fd-based openat2() validation
        // is implemented, we accept root-owned "/" as a terminal trust
        // anchor rather than checking its mode.
        if uid == 0 {
            return Ok(());
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn safe_summary_does_not_echo_trigger_contents() {
        let error = ConfigError::DuplicateTrigger {
            trigger: "secret-trigger".into(),
            first: 1,
            second: 2,
        };
        let summary = error.safe_summary();
        assert!(summary.contains("1"));
        assert!(summary.contains("2"));
        assert!(!summary.contains("secret-trigger"));
    }

    #[test]
    fn safe_summary_does_not_echo_untrusted_parent_path() {
        let error = ConfigError::InsecureParentOwner {
            path: "/home/user/private/secret-configs".into(),
            uid: 1234,
        };
        let summary = error.safe_summary();
        assert!(summary.contains("1234"));
        assert!(!summary.contains("secret-configs"));
    }

    #[test]
    fn propagate_case_uppercase_variant_colliding_with_another_trigger_is_rejected() {
        // `:sig` with propagate_case generates the matcher variant `:SIG`,
        // which collides with the second expansion's literal `:SIG`
        // trigger even though neither configured `trigger` string is a
        // literal duplicate of the other.
        let error = Config::parse(
            r#"
            [[expansion]]
            trigger = ":sig"
            replacement = "regards"
            propagate_case = true

            [[expansion]]
            trigger = ":SIG"
            replacement = "something else"
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::DuplicateTrigger { .. }));
    }

    #[test]
    fn propagate_case_capitalized_variant_colliding_with_another_trigger_is_rejected() {
        // `:sig` with propagate_case also generates `:Sig` (capitalized).
        let error = Config::parse(
            r#"
            [[expansion]]
            trigger = ":sig"
            replacement = "regards"
            propagate_case = true

            [[expansion]]
            trigger = ":Sig"
            replacement = "something else"
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::DuplicateTrigger { .. }));
    }

    #[test]
    fn propagate_case_without_collision_is_accepted() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":sig"
            replacement = "regards"
            propagate_case = true

            [[expansion]]
            trigger = ":unrelated"
            replacement = "something else"
            "#,
        )
        .unwrap();
        assert_eq!(config.expansion.len(), 2);
    }

    #[test]
    fn save_atomic_replaces_a_valid_configuration_and_keeps_private_mode() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-config-save-{}.toml", std::process::id()));
        fs::write(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"old\"\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut config = Config::load(&path).unwrap();
        config.expansion[0].replacement = "new".into();
        config.save_atomic(&path).unwrap();
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.expansion[0].replacement, "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pre_category_config_without_new_fields_still_parses() {
        // A config written before `category`/`app_filter` existed: neither
        // field is present. `#[serde(default)]` must keep this loadable
        // indefinitely -- an old config file must never fail to parse just
        // because the schema grew new optional fields.
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":legacy"
            replacement = "still works"
            description = "written before category/app_filter existed"
            tags = ["old"]
            match_mode = "immediate"
            enabled = true
            "#,
        )
        .unwrap();
        let expansion = &config.expansion[0];
        assert_eq!(expansion.trigger, ":legacy");
        assert_eq!(expansion.category, "");
        assert!(expansion.app_filter.is_empty());
    }

    #[test]
    fn app_filter_rejects_empty_entries_and_excess_count() {
        let empty_entry = Config::parse(
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [\"\"]\n",
        );
        assert!(matches!(
            empty_entry,
            Err(ConfigError::InvalidAppFilter { index: 0 })
        ));

        let too_many = format!(
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [{}]\n",
            (0..MAX_APP_FILTERS + 1)
                .map(|n| format!("\"app{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(matches!(
            Config::parse(&too_many),
            Err(ConfigError::InvalidAppFilter { index: 0 })
        ));
    }

    #[test]
    fn save_atomic_creates_missing_private_file() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-config-create-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config {
            expansion: Vec::new(),
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        };
        config.save_atomic(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().expansion.len(), 0);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(path).unwrap();
    }
}
