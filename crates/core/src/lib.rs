//! Platform-independent text expansion engine.

mod backend;
mod config;
mod engine;
mod keys;
mod matcher;
mod migration;
mod paths;
mod template;

pub use backend::{
    discover_backends, BackendKind, BackendState, BackendStatus, InjectorError, InputSource,
    InputSourceError, TextInjector,
};
pub use config::{CommandConfig, Config, ConfigError, ExpansionConfig, MatchMode, Settings};
pub use engine::{ExpansionEngine, ExpansionError, ExpansionResult, InputEvent};
pub use keys::{KeyChord, KeyChordError, Modifiers};
pub use matcher::Matcher;
pub use migration::{import_espanso, EspansoImport, MigrationError};
pub use paths::default_config_path;
pub use template::{render_template, TemplateContext, TemplateError};
