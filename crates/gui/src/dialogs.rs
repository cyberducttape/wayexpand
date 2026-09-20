use wayexpand_core::WindowContext;

/// User action deferred while an unsaved draft confirmation dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingAction {
    Select(usize),
    New,
    Duplicate,
    Delete,
    Reload,
    Undo,
    Close,
}

/// Result of the asynchronous "Use current app" detection dialog action.
#[derive(Debug)]
pub(crate) enum AppDetection {
    Found(WindowContext),
    NoWindow,
    Unavailable,
}
