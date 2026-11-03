//! Bounded, expiring context reports from opted-in local shell integrations.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use wayexpand_core::SessionContextReport;

const MAX_CONTEXTS: usize = 128;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ContextKey {
    terminal_id: String,
    session_id: String,
}

#[derive(Debug, Clone)]
struct StoredContext {
    report: SessionContextReport,
    expires_at: Instant,
    updated_at_unix: u64,
}

#[derive(Default)]
pub(crate) struct SessionContextStore {
    contexts: HashMap<ContextKey, StoredContext>,
}

impl SessionContextStore {
    pub(crate) fn set(
        &mut self,
        ttl: Duration,
        report: SessionContextReport,
    ) -> Result<(), &'static str> {
        report.validate()?;
        self.prune_expired();
        let key = ContextKey {
            terminal_id: report.terminal_id.clone(),
            session_id: report.session_id.clone(),
        };
        if !self.contexts.contains_key(&key) && self.contexts.len() >= MAX_CONTEXTS {
            return Err("context store is full; wait for an existing context to expire");
        }
        let updated_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.contexts.insert(
            key,
            StoredContext {
                report,
                expires_at: Instant::now() + ttl,
                updated_at_unix,
            },
        );
        Ok(())
    }

    pub(crate) fn get(
        &mut self,
        terminal_id: &str,
        session_id: &str,
    ) -> Option<(SessionContextReport, u64, u64)> {
        self.prune_expired();
        let key = ContextKey {
            terminal_id: terminal_id.to_owned(),
            session_id: session_id.to_owned(),
        };
        let stored = self.contexts.get(&key)?;
        Some((
            stored.report.clone(),
            stored.updated_at_unix,
            stored
                .expires_at
                .saturating_duration_since(Instant::now())
                .as_secs(),
        ))
    }

    pub(crate) fn clear(&mut self, terminal_id: &str, session_id: &str) {
        self.contexts.remove(&ContextKey {
            terminal_id: terminal_id.to_owned(),
            session_id: session_id.to_owned(),
        });
        self.prune_expired();
    }

    fn prune_expired(&mut self) {
        let now = Instant::now();
        self.contexts.retain(|_, context| context.expires_at > now);
    }
}
