use super::*;

impl ExpansionEngine {
    /// Commit undo state after an expansion result was successfully injected.
    /// Deferred callers must not record undo state before their own safety and
    /// output checks have completed.
    pub fn commit_applied_expansion(&mut self, result: &ExpansionResult) {
        self.release_deferred_match_for_result(result);
        self.record_usage(result);
        if result.undoable && result.cursor_offset.is_none() {
            self.last_expansion = Some(transaction::transaction_texts(
                &result.matched_text,
                &result.insert,
                result.reinsert_after,
            ));
        }
    }

    /// Collect applied-expansion events for local usage statistics. Hosts
    /// that never collect them lose only the oldest beyond a small bound.
    pub fn drain_usage_events(&mut self) -> Vec<crate::UsageEvent> {
        self.usage_events.drain(..).collect()
    }

    fn record_usage(&mut self, result: &ExpansionResult) {
        const MAX_PENDING_USAGE_EVENTS: usize = 1024;
        if !self.config.settings.usage_stats {
            return;
        }
        if result.snippet_id.is_empty() {
            return;
        }
        if self.usage_events.len() >= MAX_PENDING_USAGE_EVENTS {
            self.usage_events.pop_front();
        }
        self.usage_events.push_back(crate::UsageEvent {
            snippet_id: result.snippet_id.clone(),
            typed_chars: result.matched_text.chars().count(),
            inserted_chars: result.insert.chars().count(),
            unix_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs()),
        });
    }
}
