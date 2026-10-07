//! `GuiApp` behaviour, split by feature. Each module adds an `impl GuiApp`
//! block; `main.rs` keeps the shared state and composes the frame.

mod command_editor;
mod diagnostics;
mod dialogs;
mod editor;
mod editor_preview;
mod first_run;
mod import;
mod library;
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
