use crate::{render_template_with_cursor, KeyChord, TemplateContext, TemplateError};
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
    time::{SystemTime, UNIX_EPOCH},
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

    /// Require command-backed expansions and hotkeys to use absolute program
    /// paths. This avoids PATH-dependent command resolution in managed fleets.
    pub require_absolute_commands: bool,

    /// Disable title fallback for app-filtered expansions. When no compositor
    /// app ID is available, matching fails closed instead of using the
    /// user-editable window title. App IDs remain eligible for matching.
    pub disable_title_matching: bool,

    /// Refuse startup unless the selected injector can replace text as one
    /// externally atomic transaction. Enforced only in safe mode, like the
    /// other administrator-owned requirements.
    pub require_atomic_replace: bool,

    /// Refuse startup unless the selected input source reports sensitive-field
    /// focus (password, PIN, or equivalent).
    pub require_sensitive_focus: bool,

    /// Maximum replacement size in bytes. Replacements larger than this
    /// are rejected. Prevents DoS via huge expansions. 0 = unlimited.
    pub max_replacement_size: usize,

    /// Allowed output backends. If non-empty, only these backends are allowed.
    /// Examples: "libei", "input-method-v2", "wlroots", "none"
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
            require_absolute_commands: false,
            disable_title_matching: false,
            require_atomic_replace: false,
            require_sensitive_focus: false,
            max_replacement_size: 0,
            allowed_backends: Vec::new(),
            allowed_packs: Vec::new(),
            audit_prefix: "wayexpand-policy".to_string(),
        }
    }
}

impl OrganizationPolicy {
    /// Return the policy values that are allowed to block behavior.
    ///
    /// Audit mode deliberately retains the original policy for violation
    /// reporting, but contributes no enforcement values to the engine. This
    /// keeps policy detection and policy enforcement separate at every
    /// backend boundary.
    pub fn effective_enforcement_policy(&self) -> Self {
        if self.safe_mode {
            return self.clone();
        }

        let mut effective = self.clone();
        effective.disable_commands = false;
        effective.disable_hotkeys = false;
        effective.require_absolute_commands = false;
        effective.disable_title_matching = false;
        effective.require_atomic_replace = false;
        effective.require_sensitive_focus = false;
        effective.max_replacement_size = 0;
        effective.allowed_backends.clear();
        effective.allowed_packs.clear();
        effective
    }

    /// Check if any policies are active
    pub fn is_active(&self) -> bool {
        self.safe_mode
            || self.disable_commands
            || self.disable_hotkeys
            || self.require_absolute_commands
            || self.disable_title_matching
            || self.require_atomic_replace
            || self.require_sensitive_focus
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

    /// Return a human-readable capability violation for a selected deployment.
    pub fn capability_violation(
        &self,
        injector: crate::InjectorCapabilities,
        sensitive_focus: bool,
    ) -> Option<String> {
        self.capability_violation_for_source(
            injector,
            crate::InputSourceCapabilities {
                sensitive_focus,
                ..crate::InputSourceCapabilities::default()
            },
        )
    }

    /// Check guarantees against the explicit capture-source contract.
    pub fn capability_violation_for_source(
        &self,
        injector: crate::InjectorCapabilities,
        source: crate::InputSourceCapabilities,
    ) -> Option<String> {
        if self.require_atomic_replace && !injector.atomic_replace {
            return Some(
                "selected injector cannot guarantee atomic replacement transactions".into(),
            );
        }
        if self.require_sensitive_focus && !source.sensitive_focus {
            return Some(
                "selected input source cannot report password or sensitive-field focus".into(),
            );
        }
        None
    }

    /// Explain a command-path policy violation. Callers enforce the result in
    /// safe mode and log it while allowing execution in audit mode.
    pub fn command_path_violation(&self, program: &str) -> Option<String> {
        (self.require_absolute_commands && !Path::new(program).is_absolute())
            .then(|| "command program must be an absolute path by organization policy".to_string())
    }

    /// Whether a command-path policy violation should block behavior. Audit
    /// mode reports the same violation but deliberately permits execution.
    pub fn command_path_is_blocked(&self, program: &str) -> bool {
        self.safe_mode && self.command_path_violation(program).is_some()
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

    /// Return the complete, content-free explanation for an expansion policy
    /// violation. All input paths use this method so the daemon and IBus do
    /// not drift into different enforcement behavior.
    pub fn expansion_policy_violation(
        &self,
        replacement_size: usize,
        has_command: bool,
        backend: &str,
    ) -> Option<String> {
        let mut violations = Vec::new();
        if has_command && self.disable_commands {
            violations.push("command execution is disabled by organization policy".to_string());
        }
        if !self.replacement_size_allowed(replacement_size) {
            violations.push(format!(
                "replacement size {} bytes exceeds policy limit of {} bytes",
                replacement_size, self.max_replacement_size
            ));
        }
        if !self.backend_allowed(backend) {
            violations.push(format!(
                "backend '{}' is not in allowed list: {:?}",
                backend, self.allowed_backends
            ));
        }
        (!violations.is_empty()).then(|| violations.join("; "))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpansionConfig {
    /// Stable identity independent of the editable trigger. Older configs
    /// receive a UUID on load and persist it on their next save.
    #[serde(default = "new_expansion_id")]
    pub id: String,
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
    #[error("configuration {path} has insecure permissions (mode {mode:04o})")]
    InsecurePermissions { path: String, mode: u32 },
    #[error("configuration {path} is owned by uid {uid}; expected the current user or root")]
    InsecureOwner { path: String, uid: u32 },
    #[error("refusing to replace root-owned configuration {path} from an unprivileged process")]
    RootOwnedWriteRequiresAdmin { path: String },
    #[error("configuration changed since it was loaded; reload before saving")]
    RevisionConflict,
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
    #[error("expansion {index} has an invalid UUID id")]
    InvalidExpansionId { index: usize },
    #[error("expansion ids at indexes {first} and {second} are duplicated")]
    DuplicateExpansionId { first: usize, second: usize },
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
    #[error("hotkey {index} collides with settings.undo_chord ({chord:?})")]
    UndoHotkeyCollision { chord: String, index: usize },
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
            Self::RootOwnedWriteRequiresAdmin { .. } => {
                "replacing an administrator-owned config requires an administrative process".into()
            }
            Self::RevisionConflict => {
                "configuration changed externally; reload before saving".into()
            }
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
            Self::Parse(error) => parse_error_summary(error),
            Self::EmptyTrigger { index } => format!("expansion {index} has an empty trigger"),
            Self::InvalidExpansionId { index } => format!("expansion {index} has an invalid id"),
            Self::DuplicateExpansionId { first, second } => {
                format!("expansions {first} and {second} have duplicate ids")
            }
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
            Self::UndoHotkeyCollision { index, .. } => {
                format!("hotkey {index} collides with settings.undo_chord")
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
                    fs::create_dir_all(parent).map_err(|source| ConfigError::Read {
                        path: parent.display().to_string(),
                        source,
                    })?;
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(
                        |source| ConfigError::Read {
                            path: parent.display().to_string(),
                            source,
                        },
                    )?;
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
    /// descriptor read. Editors should keep the revision and use it for
    /// conditional saves rather than re-reading the file independently.
    pub fn load_versioned(path: impl AsRef<Path>) -> Result<LoadedConfig, ConfigError> {
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
        let undo_chord = self
            .settings
            .undo_chord
            .as_deref()
            .map(KeyChord::parse)
            .transpose()
            .map_err(|_| ConfigError::InvalidUndoChord)?;
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
            if binding.enabled && undo_chord.as_ref().is_some_and(|undo| undo == &chord) {
                return Err(ConfigError::UndoHotkeyCollision {
                    chord: chord.to_string(),
                    index,
                });
            }
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
            if self
                .organization
                .command_path_is_blocked(&binding.command.program)
            {
                return Err(ConfigError::InvalidHotkey {
                    index,
                    reason: "command program must be an absolute path by organization policy",
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
        let mut expansion_ids = HashMap::with_capacity(self.expansion.len());
        for (index, expansion) in self.expansion.iter().enumerate() {
            if !is_uuid(&expansion.id) {
                return Err(ConfigError::InvalidExpansionId { index });
            }
            if let Some(first) = expansion_ids.insert(expansion.id.to_ascii_lowercase(), index) {
                return Err(ConfigError::DuplicateExpansionId {
                    first,
                    second: index,
                });
            }
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
                if self.organization.command_path_is_blocked(&command.program) {
                    return Err(ConfigError::InvalidCommand {
                        index,
                        reason: "program must be an absolute path by organization policy",
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

    /// Apply an active administrator-owned policy and validate the resulting
    /// configuration. User-embedded policy remains available when no external
    /// policy is active; an active external policy is authoritative.
    pub fn apply_administrator_policy(
        &mut self,
        policy: &OrganizationPolicy,
    ) -> Result<(), ConfigError> {
        if policy.is_active() {
            // Validate the administrator input before projecting audit-only
            // values away. Invalid policy files must remain fatal even when
            // safe_mode is disabled.
            let mut candidate = self.clone();
            candidate.organization = policy.clone();
            candidate.validate()?;
            self.organization = policy.effective_enforcement_policy();
        }
        self.validate()
    }
}

/// Replace a configuration through an opened parent directory. Once the
/// directory descriptor is acquired, an attacker cannot redirect the temp
/// file or final rename by swapping a path component between validation and
/// replacement.
#[cfg(target_os = "linux")]
fn save_atomic_serialized_relative(
    resolved: &Path,
    file_name: &str,
    serialized: &[u8],
) -> Result<(), ConfigError> {
    let parent = resolved.parent().unwrap_or_else(|| Path::new("."));
    let parent_fd = open_secure_directory(parent).map_err(|source| ConfigError::Read {
        path: parent.display().to_string(),
        source,
    })?;
    let mut last_error = None;
    for attempt in 0..16 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let temp_name = format!(".{file_name}.tmp.{}.{}.{}", std::process::id(), nonce, attempt);
        let temp_fd = match rustix::fs::openat(
            &parent_fd,
            &temp_name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => fd,
            Err(error) if error == rustix::io::Errno::EXIST => {
                last_error = Some(ConfigError::Read {
                    path: temp_name,
                    source: error.into(),
                });
                continue;
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: parent.display().to_string(),
                    source: source.into(),
                });
            }
        };
        let temp_path = parent.join(&temp_name);
        let result = (|| -> Result<(), ConfigError> {
            let mut file = fs::File::from(temp_fd);
            file.write_all(serialized).map_err(|source| ConfigError::Read {
                path: temp_path.display().to_string(),
                source,
            })?;
            file.sync_all().map_err(|source| ConfigError::Read {
                path: temp_path.display().to_string(),
                source,
            })?;
            Config::validate_save_target_owner(resolved)?;
            rustix::fs::renameat(&parent_fd, &temp_name, &parent_fd, file_name).map_err(
                |source| ConfigError::Read {
                    path: resolved.display().to_string(),
                    source: source.into(),
                },
            )?;
            parent_fd
                .try_clone()
                .and_then(|directory| directory.sync_all())
                .map_err(|source| ConfigError::Read {
                    path: parent.display().to_string(),
                    source,
                })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(&parent_fd, &temp_name, rustix::fs::AtFlags::empty());
        }
        return result;
    }
    Err(last_error.unwrap_or_else(|| ConfigError::Read {
        path: parent.display().to_string(),
        source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "temporary path collision"),
    }))
}

#[cfg(target_os = "linux")]
fn open_secure_directory(path: &Path) -> std::io::Result<fs::File> {
    let root = rustix::fs::open(
        "/",
            rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let relative = path.strip_prefix("/").unwrap_or(path);
    let directory = rustix::fs::openat2(
        &root,
        relative,
        rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_SYMLINKS,
    )?;
    Ok(fs::File::from(directory))
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

fn resolve_config_target(path: &Path) -> Result<std::path::PathBuf, ConfigError> {
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
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
            Ok(parent.join(file_name))
        }
        Err(source) => Err(ConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

struct ConfigWriteLock {
    _file: fs::File,
}

impl ConfigWriteLock {
    fn acquire(target: &Path) -> Result<Self, ConfigError> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("expansions.toml");
        let lock_path = parent.join(format!(".{name}.wayexpand.lock"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|source| ConfigError::Read {
                path: lock_path.display().to_string(),
                source,
            })?;
        let metadata = file.metadata().map_err(|source| ConfigError::Read {
            path: lock_path.display().to_string(),
            source,
        })?;
        let uid = rustix::process::geteuid().as_raw();
        if !metadata.file_type().is_file() {
            return Err(ConfigError::NotRegular {
                path: lock_path.display().to_string(),
            });
        }
        if metadata.uid() != uid {
            return Err(ConfigError::InsecureOwner {
                path: lock_path.display().to_string(),
                uid: metadata.uid(),
            });
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 || metadata.nlink() != 1 {
            return Err(ConfigError::InsecurePermissions {
                path: lock_path.display().to_string(),
                mode,
            });
        }
        loop {
            // SAFETY: `file` remains alive for this guard's lifetime and owns
            // a valid descriptor. flock does not retain the pointer.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == 0 {
                return Ok(Self { _file: file });
            }
            let source = std::io::Error::last_os_error();
            if source.kind() != std::io::ErrorKind::Interrupted {
                return Err(ConfigError::Read {
                    path: lock_path.display().to_string(),
                    source,
                });
            }
        }
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
        let uid = metadata.uid();
        // Skip ownership check for system directories (/home, /) where UID
        // remapping in containers may cause unexpected ownership. User-owned
        // config directories still validate strictly.
        let is_system_dir = current == Path::new("/") || current == Path::new("/home");
        if !is_system_dir && uid != current_uid && uid != 0 {
            return Err(ConfigError::InsecureParentOwner {
                path: current.display().to_string(),
                uid,
            });
        }
        // Trust is determined by writeability, not ownership. A root-owned
        // directory with group/other write bits is still replaceable by an
        // unprivileged user and must be rejected unless sticky protection is
        // present. The filesystem root is normally 0755, so it needs no
        // special exemption.
        if !parent_mode_is_secure(mode) {
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
        // Similarly, a root-owned world-writable parent can be exploited even
        // though the child is root-owned. We validate all ancestors including
        // the root-owned filesystem root ("/"), accepting it as a terminal
        // trust anchor since the filesystem itself is the trust boundary.
        // In containerized/namespaced environments, this prevents false
        // rejections while maintaining protection against directory swaps.
        //
        // Linux save operations additionally open this parent with openat2
        // and perform temp creation/replacement relative to that descriptor.
        // The path walk remains here for portable validation and diagnostics.
        if current == Path::new("/") {
            // Reached filesystem root. Root-owned "/" is a trust anchor.
            // In systemd private namespaces, uid 65534 (overflow) may appear;
            // accept it as validation is constrained to namespace boundary.
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

/// A root-owned config only represents administrator-managed policy when an
/// unprivileged directory owner cannot replace its directory entry. Require
/// every directory in its ancestry to be root-owned; the ordinary parent
/// validator separately checks permissions and sticky-directory semantics.
fn validate_root_managed_parent_chain(path: &Path) -> Result<(), ConfigError> {
    let mut current = path.parent().unwrap_or_else(|| Path::new("/"));
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
        if !root_managed_parent_owner_allowed(metadata.uid()) {
            return Err(ConfigError::InsecureParentOwner {
                path: current.display().to_string(),
                uid: metadata.uid(),
            });
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

fn root_managed_parent_owner_allowed(uid: u32) -> bool {
    uid == 0
}

fn root_owned_target_requires_admin(target_uid: u32, current_uid: u32) -> bool {
    target_uid == 0 && current_uid != 0
}

fn parent_mode_is_secure(mode: u32) -> bool {
    mode & 0o022 == 0 || mode & 0o1000 != 0
}

/// Describe a TOML/schema error by location only. The `Display` form of a
/// `toml` error quotes the offending source line, which can be snippet
/// content, so only its "line N, column M" header is kept. Missing-field
/// errors are named because serde reports schema field names there, never
/// user-provided text.
fn parse_error_summary(error: &toml::de::Error) -> String {
    let rendered = error.to_string();
    let location = rendered
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("TOML parse error at "))
        .filter(|location| {
            location.starts_with("line ")
                && location
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || " ,".contains(character))
        });
    let missing_field = error
        .message()
        .strip_prefix("missing field `")
        .and_then(|rest| rest.strip_suffix('`'))
        .filter(|field| {
            field
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_')
        });
    let mut summary = String::from("invalid TOML");
    if let Some(location) = location {
        summary.push_str(" at ");
        summary.push_str(location);
    }
    if let Some(field) = missing_field {
        summary.push_str(&format!(" (missing field `{field}`)"));
    }
    summary
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
    fn root_owned_world_writable_parent_mode_is_not_trusted() {
        // Ownership cannot make a directory safe when its mode grants write
        // access to group/other users; this is the regression behind the
        // parent-directory validation fix.
        assert!(!parent_mode_is_secure(0o0777));
        assert!(parent_mode_is_secure(0o1777));
        assert!(parent_mode_is_secure(0o0755));
    }

    #[test]
    fn root_managed_configuration_requires_root_owned_parents() {
        assert!(root_managed_parent_owner_allowed(0));
        assert!(!root_managed_parent_owner_allowed(1000));
    }

    #[test]
    fn generic_config_save_requires_admin_for_root_owned_target() {
        assert!(root_owned_target_requires_admin(0, 1000));
        assert!(!root_owned_target_requires_admin(0, 0));
        assert!(!root_owned_target_requires_admin(1000, 1000));
        assert!(!root_owned_target_requires_admin(1001, 1000));
    }

    #[test]
    fn root_owned_config_below_user_owned_directory_is_rejected() {
        let uid = rustix::process::geteuid().as_raw();
        let parent = std::env::temp_dir().join(format!(
            "wayexpand-root-config-parent-{}-{}",
            std::process::id(),
            EXPANSION_ID_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&parent).unwrap();
        if uid != 0 {
            assert!(matches!(
                validate_root_managed_parent_chain(&parent.join("expansions.toml")),
                Err(ConfigError::InsecureParentOwner { uid: owner, .. }) if owner == uid
            ));
        }
        fs::remove_dir(parent).unwrap();
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
    fn versioned_load_uses_one_source_snapshot_and_rejects_stale_save() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-config-revision-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let source =
            "# retained source snapshot\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"old\"\n";
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let loaded = Config::load_versioned(&path).unwrap();
        assert_eq!(loaded.source(), source);
        let mut stale_candidate = loaded.config.clone();
        stale_candidate.expansion[0].replacement = "stale edit".into();

        let mut external = loaded.config.clone();
        external.expansion[0].replacement = "external edit".into();
        external.save_atomic(&path).unwrap();
        let error = stale_candidate
            .save_atomic_if_revision_matches(&path, &loaded.revision)
            .unwrap_err();
        assert!(matches!(error, ConfigError::RevisionConflict));
        assert_eq!(
            Config::load(&path).unwrap().expansion[0].replacement,
            "external edit"
        );

        let filename = path.file_name().unwrap().to_string_lossy();
        let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
        fs::remove_file(path).unwrap();
        fs::remove_file(lock_path).unwrap();
    }

    #[test]
    fn concurrent_conditional_writers_allow_only_one_revision_winner() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-config-concurrent-revision-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let initial =
            Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"initial\"\n").unwrap();
        initial.save_atomic(&path).unwrap();
        let loaded = Config::load_versioned(&path).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let mut first = loaded.config.clone();
        first.expansion[0].replacement = "first writer".into();
        let first_barrier = barrier.clone();
        let first_path = path.clone();
        let first_revision = loaded.revision.clone();
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            first
                .save_atomic_if_revision_matches(first_path, &first_revision)
                .is_ok()
        });

        let mut second = loaded.config;
        second.expansion[0].replacement = "second writer".into();
        let second_barrier = barrier;
        let second_path = path.clone();
        let second_revision = loaded.revision;
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            second
                .save_atomic_if_revision_matches(second_path, &second_revision)
                .is_ok()
        });

        assert_ne!(first.join().unwrap(), second.join().unwrap());
        let saved = Config::load(&path).unwrap();
        assert!(matches!(
            saved.expansion[0].replacement.as_str(),
            "first writer" | "second writer"
        ));
        let filename = path.file_name().unwrap().to_string_lossy();
        let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
        fs::remove_file(path).unwrap();
        fs::remove_file(lock_path).unwrap();
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
        assert!(is_uuid(&expansion.id));
    }

    #[test]
    fn generated_expansion_ids_are_unique_and_round_trip() {
        let config = Config::parse(
            "[[expansion]]\ntrigger=':one'\nreplacement='one'\n[[expansion]]\ntrigger=':two'\nreplacement='two'\n",
        )
        .unwrap();
        let first = config.expansion[0].id.clone();
        let second = config.expansion[1].id.clone();
        assert!(is_uuid(&first));
        assert_ne!(first, second);

        let encoded = toml::to_string(&config).unwrap();
        let decoded = Config::parse(&encoded).unwrap();
        assert_eq!(decoded.expansion[0].id, first);
        assert_eq!(decoded.expansion[1].id, second);
    }

    #[test]
    fn expansion_ids_must_be_valid_and_unique() {
        let invalid =
            Config::parse("[[expansion]]\nid='not-a-uuid'\ntrigger=':one'\nreplacement='one'\n")
                .unwrap_err();
        assert!(matches!(
            invalid,
            ConfigError::InvalidExpansionId { index: 0 }
        ));

        let repeated = "[[expansion]]\nid='00000000-0000-4000-8000-000000000001'\ntrigger=':one'\nreplacement='one'\n[[expansion]]\nid='00000000-0000-4000-8000-000000000001'\ntrigger=':two'\nreplacement='two'\n";
        assert!(matches!(
            Config::parse(repeated),
            Err(ConfigError::DuplicateExpansionId {
                first: 0,
                second: 1
            })
        ));
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
    fn organization_policy_can_require_absolute_command_paths() {
        let error = Config::parse(
            r#"
            [organization]
            safe_mode = true
            require_absolute_commands = true

            [[expansion]]
            trigger = ":git"
            replacement = ""
            command = { program = "git", args = ["status"] }
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigError::InvalidCommand { index: 0, .. }
        ));

        let config = Config::parse(
            r#"
            [organization]
            safe_mode = true
            require_absolute_commands = true

            [[expansion]]
            trigger = ":git"
            replacement = ""
            command = { program = "/usr/bin/git", args = ["status"] }
            "#,
        )
        .unwrap();
        assert_eq!(
            config.expansion[0].command.as_ref().unwrap().program,
            "/usr/bin/git"
        );
    }

    #[test]
    fn organization_policy_can_require_absolute_hotkey_command_paths() {
        let error = Config::parse(
            r#"
            [organization]
            safe_mode = true
            require_absolute_commands = true

            [[hotkey]]
            chord = "Ctrl+Alt+T"
            command = { program = "konsole" }
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::InvalidHotkey { index: 0, .. }));
    }

    #[test]
    fn undo_chord_cannot_collide_with_an_enabled_hotkey() {
        let error = Config::parse(
            r#"
            [settings]
            undo_chord = "control + z"

            [[hotkey]]
            chord = "Ctrl+Z"
            command = { program = "/bin/true" }
            "#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ConfigError::UndoHotkeyCollision { index: 0, chord } if chord == "Ctrl+Z"
        ));
    }

    #[test]
    fn disabled_hotkey_may_match_the_undo_chord() {
        let config = Config::parse(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[hotkey]]
            enabled = false
            chord = "control+z"
            command = { program = "/bin/true" }
            "#,
        )
        .unwrap();

        assert!(!config.hotkey[0].enabled);
    }

    #[test]
    fn audit_policy_allows_relative_command_paths() {
        let config = Config::parse(
            r#"
            [organization]
            safe_mode = false
            require_absolute_commands = true

            [[expansion]]
            trigger = ":git"
            replacement = ""
            command = { program = "git", args = ["status"] }

            [[hotkey]]
            chord = "Ctrl+Alt+T"
            command = { program = "konsole" }
            "#,
        )
        .unwrap();
        assert!(config.organization.require_absolute_commands);
    }

    #[test]
    fn command_path_violation_blocks_only_in_safe_mode() {
        let audit = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        assert!(audit.command_path_violation("git").is_some());
        assert!(!audit.command_path_is_blocked("git"));

        let safe = OrganizationPolicy {
            safe_mode: true,
            ..audit.clone()
        };
        assert!(safe.command_path_violation("git").is_some());
        assert!(safe.command_path_is_blocked("git"));
        assert!(!safe.command_path_is_blocked("/usr/bin/git"));
    }

    #[test]
    fn capability_requirements_distinguish_atomic_and_sensitive_guarantees() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_atomic_replace: true,
            require_sensitive_focus: true,
            ..OrganizationPolicy::default()
        };
        let conservative = crate::InjectorCapabilities::default();
        assert_eq!(
            policy.capability_violation(conservative, false).as_deref(),
            Some("selected injector cannot guarantee atomic replacement transactions")
        );

        let atomic = crate::InjectorCapabilities {
            atomic_replace: true,
            ..crate::InjectorCapabilities::default()
        };
        assert_eq!(
            policy.capability_violation(atomic, false).as_deref(),
            Some("selected input source cannot report password or sensitive-field focus")
        );
        assert!(policy.capability_violation(atomic, true).is_none());
        assert!(policy
            .capability_violation_for_source(
                atomic,
                crate::InputSourceCapabilities {
                    sensitive_focus: true,
                    exclusive_capture: true,
                    ..crate::InputSourceCapabilities::default()
                }
            )
            .is_none());
        assert!(policy
            .capability_violation_for_source(
                atomic,
                crate::InputSourceCapabilities {
                    exclusive_capture: true,
                    ..crate::InputSourceCapabilities::default()
                }
            )
            .is_some_and(|violation| violation.contains("sensitive-field focus")));
    }

    #[test]
    fn audit_policy_has_no_effective_enforcement_values() {
        let policy = OrganizationPolicy {
            safe_mode: false,
            disable_commands: true,
            disable_hotkeys: true,
            require_absolute_commands: true,
            disable_title_matching: true,
            max_replacement_size: 256,
            allowed_backends: vec!["none".into()],
            allowed_packs: vec!["managed".into()],
            ..OrganizationPolicy::default()
        };
        let effective = policy.effective_enforcement_policy();

        assert!(!effective.disable_commands);
        assert!(!effective.disable_hotkeys);
        assert!(!effective.require_absolute_commands);
        assert!(!effective.disable_title_matching);
        assert!(!effective.require_atomic_replace);
        assert!(!effective.require_sensitive_focus);
        assert_eq!(effective.max_replacement_size, 0);
        assert!(effective.allowed_backends.is_empty());
        assert!(effective.allowed_packs.is_empty());
        assert!(policy
            .expansion_policy_violation(512, false, "wayland")
            .is_some());
    }

    #[test]
    fn applying_audit_policy_keeps_engine_limits_unrestricted() {
        let mut config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":ok"
            replacement = "ok"
            "#,
        )
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            max_replacement_size: 256,
            ..OrganizationPolicy::default()
        };

        config.apply_administrator_policy(&policy).unwrap();
        assert_eq!(config.organization.max_replacement_size, 0);
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
