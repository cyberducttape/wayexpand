//! Platform-independent text expansion engine.

mod backend;
mod capabilities;
mod config;
mod engine;
mod fleet;
mod keys;
mod matcher;
mod migration;
mod paths;
mod store;
mod template;

pub use backend::{
    discover_backends, BackendKind, BackendState, BackendStatus, InjectorError, InputSource,
    InputSourceError, TextInjector, WindowTracker, WindowTrackerError,
};
pub use capabilities::{all_capabilities, Capabilities, TextMethod};
pub use config::{
    CommandConfig, CommandEnvironment, Config, ConfigError, ExpansionConfig, FontScale,
    HotkeyConfig, MatchMode, OrganizationPolicy, Settings,
};
pub use engine::{
    run_command, CommandError, CommandMetrics, ExpansionEngine, ExpansionError, ExpansionResult,
    HotkeyError, HotkeyResult, InputEvent, WindowContext,
};
pub use fleet::{FleetConfig, FleetError, Layer, MergeStats, Provenance};
pub use keys::{KeyChord, KeyChordError, Modifiers};
pub use matcher::Matcher;
pub use migration::{import_espanso, EspansoImport, MigrationError};
pub use paths::default_config_path;
pub use store::ConfigStore;
pub use template::{render_template, render_template_with_cursor, TemplateContext, TemplateError};
