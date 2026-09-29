//! Input event processing and expansion computation.
//!
//! Processes input events (text, keys, delimiters, window changes) through the
//! expansion engine, managing buffer state, window context, and pause/focus state.

use super::{ExpansionEngine, ExpansionResult, InputEvent, PendingExpansionResult};

impl ExpansionEngine {
    /// Process an input event through the expansion state machine.
    ///
    /// The public entry point lives in this module so event processing remains
    /// separated from matching policy, transaction handling, and command
    /// runtime ownership as those implementations continue to be extracted.
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        self.process_internal(event)
    }

    /// Process input while reserving command-backed matches for caller policy
    /// approval and later dispatch.
    pub fn process_deferred(&mut self, event: InputEvent) -> Vec<PendingExpansionResult> {
        self.process_deferred_internal(event)
    }
}
