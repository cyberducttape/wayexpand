use super::*;

impl ExpansionEngine {
    /// Forget the deferred trigger reservation after an output transaction
    /// whose effect is uncertain. The trigger may already have been replaced
    /// in the focused application, so restoring it before the next input
    /// event could replay the expansion into that application a second time.
    /// Clear its provisional undo record as well: output was not verified, so
    /// a later undo must not erase unrelated target text. This does not
    /// record usage or create undo state.
    pub fn discard_deferred_result(&mut self, result: &ExpansionResult) {
        if self
            .take_deferred_reservation(Self::reserved_text(result))
            .is_some()
        {
            self.discard_undo();
        }
    }

    /// Commit undo state after the host has verified the replacement in the
    /// target application. Backend acknowledgment alone is insufficient;
    /// callers that only know output was emitted must use
    /// [`Self::commit_emitted_unverified_expansion`].
    pub fn commit_applied_expansion(&mut self, result: &ExpansionResult) {
        self.release_deferred_match_for_result(result);
        self.record_usage(result);
        if result.undoable && result.cursor_offset.is_none() && self.validated.undo_chord.is_some()
        {
            self.clear_undo_state();
            self.last_expansion = Some(transaction::transaction_texts(
                &result.matched_text,
                &result.insert,
                result.reinsert_after,
            ));
        }
    }

    /// Finish an output whose backend acknowledged emission but cannot verify
    /// that the application changed its document. Count the emitted
    /// expansion for usage reporting, release its deferred matcher state, and
    /// clear any older undo record because the document may now differ from
    /// the text that record describes. Never create undo state from an
    /// unverified application result.
    pub fn commit_emitted_unverified_expansion(&mut self, result: &ExpansionResult) {
        self.release_deferred_match_for_result(result);
        self.record_usage(result);
        self.discard_undo();
    }

    /// Collect applied-expansion events for local usage statistics. Hosts
    /// that never collect them lose only the oldest beyond a small bound.
    pub fn drain_usage_events(&mut self) -> Vec<crate::UsageEvent> {
        self.usage_events
            .drain(..)
            .map(|(_, event)| event)
            .collect()
    }

    /// Drain applied-expansion events with their process-local ordering keys.
    /// Used by hosts that need an exact boundary for a concurrent clear.
    pub fn drain_usage_events_sequenced(&mut self) -> Vec<(u64, crate::UsageEvent)> {
        self.usage_events.drain(..).collect()
    }

    /// Return and reset usage events evicted from the bounded engine buffer.
    pub fn take_dropped_usage_event_count(&mut self) -> u64 {
        std::mem::take(&mut self.usage_events_dropped)
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
            self.usage_events_dropped = self.usage_events_dropped.saturating_add(1);
        }
        let sequence = crate::usage::next_usage_event_sequence();
        self.usage_events.push_back((
            sequence,
            crate::UsageEvent {
                snippet_id: result.snippet_id.to_string(),
                typed_chars: result.matched_text.chars().count(),
                inserted_chars: result.insert.chars().count(),
                unix_timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_secs()),
            },
        ));
    }
}
