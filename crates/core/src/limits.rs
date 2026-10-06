//! Every size, count, and time limit that more than one component depends
//! on. Validation, the engine, the daemon, the GUI, and the CLI read these
//! values from here, so a limit can never be changed in one place and not
//! another. Limits used by a single module only stay next to their code.

// ---- Configuration ----------------------------------------------------

/// Largest configuration file accepted.
pub const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;
/// Snippets per configuration.
pub const MAX_EXPANSIONS: usize = 10_000;
/// Hotkeys per configuration.
pub const MAX_HOTKEYS: usize = 1_024;
pub const MAX_HOTKEY_DESCRIPTION_CHARS: usize = 256;
pub const MAX_DESCRIPTION_CHARS: usize = 512;
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_CATEGORY_CHARS: usize = 64;
pub const MAX_APP_FILTERS: usize = 32;
pub const MAX_APP_FILTER_CHARS: usize = 256;
pub const MAX_TEMPLATE_ENV: usize = 32;

// ---- Triggers ---------------------------------------------------------

/// Unicode scalars in one trigger or alias.
pub const MAX_TRIGGER_CHARS: usize = 128;
/// Aliases per snippet.
pub const MAX_ALIASES: usize = 32;
/// Scalars across all configured triggers and aliases.
pub const MAX_TOTAL_TRIGGER_CHARS: usize = 256 * 1024;
/// Scalars across all effective triggers (after case and NFC/NFD variants).
pub const MAX_EFFECTIVE_TRIGGER_SCALARS: usize = 1_000_000;
/// Effective triggers in the matcher.
pub const MAX_EFFECTIVE_TRIGGERS: usize = 100_000;

// ---- Replacements and templates ----------------------------------------

/// Bytes in one snippet's replacement template.
pub const MAX_REPLACEMENT_BYTES: usize = 1024 * 1024;
/// Bytes a rendered template may expand to.
pub const MAX_RENDERED_BYTES: usize = 1024 * 1024;

// ---- Commands -----------------------------------------------------------

/// Limits enforced by [`crate::validate_command_config`], the single
/// authority for command settings. Front ends may display them but must
/// validate through that function rather than re-implementing the checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandLimits {
    pub max_args: usize,
    pub max_program_chars: usize,
    pub max_arg_chars: usize,
    pub max_arg_data_chars: usize,
    pub max_env_vars: usize,
    pub max_env_name_chars: usize,
    pub max_timeout_ms: u64,
    pub max_cache_ms: u64,
}

pub const COMMAND_LIMITS: CommandLimits = CommandLimits {
    max_args: 32,
    max_program_chars: 256,
    max_arg_chars: 1024,
    max_arg_data_chars: 16 * 1024,
    max_env_vars: 32,
    max_env_name_chars: 256,
    max_timeout_ms: 5_000,
    max_cache_ms: 60_000,
};

/// Bytes of command stdout used as a replacement.
pub const MAX_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;

// ---- Forms ----------------------------------------------------------------

/// Bytes a snippet form helper may return.
pub const MAX_FORM_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
/// Bytes in one submitted form value.
pub const MAX_FORM_VALUE_BYTES: usize = 64 * 1024;

// ---- Output ---------------------------------------------------------------

/// Expansion results produced by one input event.
pub const MAX_RESULTS_PER_EVENT: usize = 1024;
/// Bytes of replacement text produced by one input event.
pub const MAX_RESULT_BYTES_PER_EVENT: usize = 4 * 1024 * 1024;

// ---- Packs ----------------------------------------------------------------

pub const MAX_PACK_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PACK_SNIPPET_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_PACK_SNIPPET_FILES: usize = 1024;
pub const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

// ---- IPC ------------------------------------------------------------------

/// Largest control-socket response a client reads. Shared by the CLI, TUI,
/// and GUI clients; the daemon's status body is tested to fit within it.
pub const CONTROL_MAX_RESPONSE_BYTES: usize = 4096;

// ---- Local state ------------------------------------------------------------

/// Largest usage-statistics file read back.
pub const MAX_USAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
