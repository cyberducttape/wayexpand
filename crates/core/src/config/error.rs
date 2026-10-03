//! Configuration errors and their content-free summaries.

use super::*;

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
    #[error("configuration {path} is busy; another writer holds its lock")]
    Busy { path: String },
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
            Self::Busy { .. } => "configuration is busy; another writer holds its lock".into(),
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

/// Describe a TOML/schema error by location only. The `Display` form of a
/// `toml` error quotes the offending source line, which can be snippet
/// content, so only its "line N, column M" header is kept. Missing-field
/// errors are named because serde reports schema field names there, never
/// user-provided text.
pub(super) fn parse_error_summary(error: &toml::de::Error) -> String {
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
