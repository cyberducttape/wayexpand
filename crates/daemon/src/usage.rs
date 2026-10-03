//! Persists the engine's local usage events (see `wayexpand_core::usage`).
//! Events are collected every reactor turn and written at most once a
//! minute, plus once at shutdown, so typing never waits on the disk. Each
//! save merges the unsaved events into a fresh read of the file, so
//! `wayexpand stats --clear` takes effect while the daemon runs.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use tracing::warn;
use wayexpand_core::{ExpansionEngine, UsageEvent, UsageStats};

const SAVE_INTERVAL: Duration = Duration::from_secs(60);
const MAX_PENDING_EVENTS: usize = 100_000;

pub struct UsageRecorder {
    path: PathBuf,
    pending: Vec<UsageEvent>,
    last_save: Instant,
}

impl UsageRecorder {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            pending: Vec::new(),
            last_save: Instant::now(),
        }
    }

    /// Collect pending events from the engine; save when due.
    pub fn collect(&mut self, engine: &mut ExpansionEngine) {
        self.pending.extend(engine.drain_usage_events());
        if self.pending.len() > MAX_PENDING_EVENTS {
            let excess = self.pending.len() - MAX_PENDING_EVENTS;
            self.pending.drain(..excess);
        }
        if !self.pending.is_empty() && self.last_save.elapsed() >= SAVE_INTERVAL {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.last_save = Instant::now();
        let mut stats = UsageStats::load(&self.path);
        for event in &self.pending {
            stats.record(event);
        }
        match stats.save(&self.path) {
            Ok(()) => self.pending.clear(),
            Err(error) => warn!(%error, "could not save local usage statistics"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_merges_into_the_file_and_honours_a_clear() {
        let directory =
            std::env::temp_dir().join(format!("wayexpand-usage-recorder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("usage-stats.json");
        let event = |id: &str| UsageEvent {
            snippet_id: id.into(),
            typed_chars: 4,
            inserted_chars: 20,
            unix_timestamp: 1_700_000_000,
        };
        let mut recorder = UsageRecorder::new(path.clone());
        recorder.pending.push(event("a"));
        recorder.flush();
        assert_eq!(UsageStats::load(&path).snippets["a"].count, 1);

        // A clear (file removed) is respected by the next save.
        std::fs::remove_file(&path).unwrap();
        recorder.pending.push(event("b"));
        recorder.flush();
        let stats = UsageStats::load(&path);
        assert!(!stats.snippets.contains_key("a"));
        assert_eq!(stats.snippets["b"].count, 1);
        let _ = std::fs::remove_dir_all(directory);
    }
}
