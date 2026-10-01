//! Unix-only text expansion engine and secure configuration loader.

/// Version of the daemon's stable key/value control-socket status contract.
/// Increment when a consumer must distinguish a newer required field set.
pub const CONTROL_STATUS_SCHEMA: u32 = 1;

#[cfg(not(unix))]
compile_error!("wayexpand-core currently requires a Unix target");

mod backend;
mod capabilities;
mod config;
mod engine;
mod fleet;
mod keys;
mod matcher;
mod migration;
mod pack;
mod paths;
mod policy;
mod store;
mod template;

pub use backend::{
    discover_backends, BackendKind, BackendState, BackendStatus, InjectorCapabilities,
    InjectorError, InjectorErrorKind, InputSource, InputSourceCapabilities, InputSourceError,
    KeyEventState, TextInjector, WindowTracker, WindowTrackerError,
};
pub use capabilities::{all_capabilities, Capabilities, TextMethod};
pub use config::{
    AppFilter, CommandConfig, CommandEnvironment, Config, ConfigError, ConfigRevision,
    ExpansionConfig, FontScale, HotkeyConfig, LoadedConfig, MatchMode, OrganizationPolicy,
    Settings,
};
pub use engine::{
    run_command, run_command_cancellable, CommandError, CommandMetrics, ExpansionEngine,
    ExpansionError, ExpansionResult, HotkeyError, HotkeyResult, InputEvent, InsertError,
    PendingExpansionDispatch, PendingExpansionResult, WindowContext,
};
pub use fleet::{FleetConfig, FleetError, Layer, MergeStats, Provenance};
pub use keys::{KeyChord, KeyChordError, Modifiers};
pub use matcher::Matcher;
pub use migration::{
    import_espanso, EspansoImport, EspansoImportReport, EspansoImportWarning,
    EspansoUnsupportedMatch, MigrationError,
};
pub use pack::{import_pack, inspect_pack, PackError, PackInspection, PackManifest};
pub use paths::default_config_path;
pub use policy::{
    load_organization_policy, load_organization_policy_from_paths, parse_organization_policy,
    policy_backend_name, pre_flight_check, validate_organization_policy_directory,
    validate_organization_policy_file, MAX_ORGANIZATION_POLICY_BYTES, ORGANIZATION_POLICY_DIR,
    ORGANIZATION_POLICY_PATH,
};
pub use store::{ConfigStore, ConfigStoreStatus};
pub use template::{render_template, render_template_with_cursor, TemplateContext, TemplateError};
