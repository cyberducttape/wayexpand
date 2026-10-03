//! `GuiApp` behaviour, split by feature. Each module adds an `impl GuiApp`
//! block; `main.rs` keeps the shared state and composes the frame.

mod diagnostics;
mod import;
mod settings;
