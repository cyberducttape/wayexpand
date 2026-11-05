//! One function per `wayexpand` subcommand; `run` dispatches to them.

pub(crate) mod check;
pub(crate) mod compatibility;
pub(crate) mod config;
pub(crate) mod config_inspect;
pub(crate) mod context;
pub(crate) mod daemon;
pub(crate) mod debug;
pub(crate) mod health;
pub(crate) mod ibus;
pub(crate) mod import_add;
pub(crate) mod install;
pub(crate) mod library;
pub(crate) mod metrics;
pub(crate) mod new;
pub(crate) mod pack_lock;
pub(crate) mod pack_quality;
pub(crate) mod pack_trust;
pub(crate) mod profile;
pub(crate) mod reset;
pub(crate) mod restore;
pub(crate) mod service;
pub(crate) mod system;
pub(crate) mod workflow;

/// The remaining command-line arguments after the subcommand name.
pub(crate) type Args = std::iter::Skip<std::env::Args>;
