//! One function per `wayexpand` subcommand; `run` dispatches to them.

pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod system;

/// The remaining command-line arguments after the subcommand name.
pub(crate) type Args = std::iter::Skip<std::env::Args>;
