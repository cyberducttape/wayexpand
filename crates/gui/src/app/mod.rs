//! `GuiApp` behaviour, split by feature. Each module adds an `impl GuiApp`
//! block; `main.rs` keeps the shared state and composes the frame.

mod diagnostics;
mod dialogs;
mod import;
mod library;
mod settings;
mod setup;
mod toolbar;

pub(crate) use setup::onboarding_step;
mod snippets;
