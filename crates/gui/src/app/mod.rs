//! `GuiApp` behaviour, split by feature. Each module adds an `impl GuiApp`
//! block; `main.rs` keeps the shared state and composes the frame.

mod command_editor;
mod diagnostics;
mod dialogs;
mod editor;
pub(crate) use editor::FormFieldError;
mod editor_preview;
mod first_run;
mod import;
mod library;
mod next_step;
pub(crate) use next_step::{Action as NextAction, Problem};
mod pack;
mod profile;
pub(crate) mod route_fit;
mod runtime_events;
mod saving;
mod settings;
mod setup;
mod snippets;
mod status_bar;
mod sync;
mod toolbar;

#[cfg(test)]
pub(crate) use editor::insert_at_char_range;
pub(crate) use saving::UndoEntry;
