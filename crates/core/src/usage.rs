//! Local-only usage statistics.
//!
//! Records which snippets expand, by stable snippet ID, never typed or
//! inserted text: counts, character totals, last use, and per-day totals.
//! The file lives next to the configuration (mode 0600) and is never sent
//! anywhere. `settings.usage_stats = false` turns recording off.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{Config, MatchMode};

const USAGE_FILE: &str = "usage-stats.json";
use crate::limits::MAX_USAGE_FILE_BYTES;
const MAX_DAILY_ENTRIES: usize = 400;
const MAX_SNIPPET_ENTRIES: usize = 20_000;
const SECONDS_PER_DAY: u64 = 86_400;

/// One applied expansion, as recorded by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEvent {
    pub snippet_id: String,
    /// Characters the user typed for the trigger.
    pub typed_chars: usize,
    /// Characters the expansion inserted.
    pub inserted_chars: usize,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetUsage {
    pub count: u64,
    pub chars_typed: u64,
    pub chars_inserted: u64,
    pub last_used: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStats {
    #[serde(default)]
    pub snippets: BTreeMap<String, SnippetUsage>,
    /// Expansions per UTC day (`YYYY-MM-DD`).
    #[serde(default)]
    pub daily: BTreeMap<String, u64>,
}

/// Where usage statistics for `config_path` are kept.
pub fn usage_stats_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(USAGE_FILE)
}

impl UsageStats {
    /// Load statistics; a missing, oversized, or unreadable file starts fresh
    /// rather than blocking expansion.
    pub fn load(path: &Path) -> Self {
        let Ok(metadata) = fs::metadata(path) else {
            return Self::default();
        };
        if metadata.len() > MAX_USAGE_FILE_BYTES {
            return Self::default();
        }
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn record(&mut self, event: &UsageEvent) {
        let entry = self.snippets.entry(event.snippet_id.clone()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.chars_typed = entry.chars_typed.saturating_add(event.typed_chars as u64);
        entry.chars_inserted = entry
            .chars_inserted
            .saturating_add(event.inserted_chars as u64);
        entry.last_used = entry.last_used.max(event.unix_timestamp);
        let day = crate::template::format_date(event.unix_timestamp);
        let total = self.daily.entry(day).or_default();
        *total = total.saturating_add(1);
        self.enforce_limits();
    }

    /// Merge pre-aggregated counters without reconstructing individual
    /// events. Used by the daemon's asynchronous persistence worker.
    pub fn merge(&mut self, delta: &UsageStats) {
        for (id, incoming) in &delta.snippets {
            let usage = self.snippets.entry(id.clone()).or_default();
            usage.count = usage.count.saturating_add(incoming.count);
            usage.chars_typed = usage.chars_typed.saturating_add(incoming.chars_typed);
            usage.chars_inserted = usage.chars_inserted.saturating_add(incoming.chars_inserted);
            usage.last_used = usage.last_used.max(incoming.last_used);
        }
        for (day, count) in &delta.daily {
            let total = self.daily.entry(day.clone()).or_default();
            *total = total.saturating_add(*count);
        }
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        while self.daily.len() > MAX_DAILY_ENTRIES {
            self.daily.pop_first();
        }
        if self.snippets.len() > MAX_SNIPPET_ENTRIES {
            // Drop the least recently used entries first.
            let mut by_age: Vec<(u64, String)> = self
                .snippets
                .iter()
                .map(|(id, usage)| (usage.last_used, id.clone()))
                .collect();
            by_age.sort();
            for (_, id) in by_age
                .into_iter()
                .take(self.snippets.len() - MAX_SNIPPET_ENTRIES)
            {
                self.snippets.remove(&id);
            }
        }
    }

    /// Write atomically with mode 0600.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = parent.join(format!(".{USAGE_FILE}.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temporary, path)
    }

    /// Summarize statistics against the current library.
    pub fn report(&self, config: &Config, now: u64, days: u64) -> UsageReport {
        let since = now.saturating_sub(days.saturating_mul(SECONDS_PER_DAY));
        let since_day = crate::template::format_date(since);
        let expansions = self
            .daily
            .iter()
            .filter(|(day, _)| day.as_str() >= since_day.as_str())
            .map(|(_, count)| *count)
            .sum();
        let mut top: Vec<UsageLine> = Vec::new();
        let mut unused = Vec::new();
        let mut keystrokes_avoided: u64 = 0;
        let unused_before = now.saturating_sub(90 * SECONDS_PER_DAY);
        for expansion in config
            .expansion
            .iter()
            .filter(|expansion| expansion.enabled)
        {
            match self.snippets.get(&expansion.id) {
                Some(usage) => {
                    keystrokes_avoided = keystrokes_avoided
                        .saturating_add(usage.chars_inserted.saturating_sub(usage.chars_typed));
                    top.push(UsageLine {
                        trigger: expansion.trigger.clone(),
                        count: usage.count,
                        last_used: usage.last_used,
                    });
                    if usage.last_used < unused_before {
                        unused.push(expansion.trigger.clone());
                    }
                }
                None => unused.push(expansion.trigger.clone()),
            }
        }
        top.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.trigger.cmp(&right.trigger))
        });
        top.truncate(10);
        UsageReport {
            days,
            expansions,
            keystrokes_avoided,
            top,
            unused_90_days: unused,
            trigger_risks: trigger_risks(config),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageLine {
    pub trigger: String,
    pub count: u64,
    pub last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageReport {
    pub days: u64,
    /// Expansions in the last `days` days.
    pub expansions: u64,
    /// Inserted minus typed characters, over all recorded use.
    pub keystrokes_avoided: u64,
    pub top: Vec<UsageLine>,
    /// Enabled snippets never used, or not used in 90 days.
    pub unused_90_days: Vec<String>,
    pub trigger_risks: Vec<TriggerRisk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TriggerRisk {
    pub trigger: String,
    pub reason: String,
}

/// Triggers likely to expand by accident or to shadow each other.
pub fn trigger_risks(config: &Config) -> Vec<TriggerRisk> {
    let enabled: Vec<_> = config
        .expansion
        .iter()
        .filter(|expansion| expansion.enabled)
        .collect();
    let mut all_triggers: Vec<String> = enabled
        .iter()
        .flat_map(|expansion| {
            std::iter::once(expansion.trigger.clone()).chain(expansion.aliases.clone())
        })
        .collect();
    all_triggers.sort_unstable();
    let mut risks = Vec::new();
    for expansion in &enabled {
        for trigger in std::iter::once(&expansion.trigger).chain(&expansion.aliases) {
            let plain_word = trigger.chars().all(char::is_alphabetic);
            if plain_word && expansion.match_mode == MatchMode::Immediate {
                risks.push(TriggerRisk {
                    trigger: trigger.clone(),
                    reason: "ordinary letters in immediate mode: it fires inside words that \
                             contain it; add a prefix like ';' or use word-boundary mode"
                        .into(),
                });
            } else if trigger.chars().count() <= 2 && plain_word {
                risks.push(TriggerRisk {
                    trigger: trigger.clone(),
                    reason: "very short: likely to collide with ordinary typing".into(),
                });
            }
            if expansion.match_mode == MatchMode::Immediate {
                // Prefixes form a contiguous range in sorted lexicographic
                // order. The first entry after `trigger` is enough to detect
                // whether that range contains a longer trigger, reducing
                // this check from O(T²) to O(T log T).
                let next_index = all_triggers
                    .partition_point(|candidate| candidate.as_str() <= trigger.as_str());
                if let Some(longer) = all_triggers.get(next_index).filter(|candidate| {
                    candidate.len() > trigger.len() && candidate.starts_with(trigger.as_str())
                }) {
                    risks.push(TriggerRisk {
                        trigger: trigger.clone(),
                        reason: format!(
                            "is the start of {longer}, so it waits for the next key before expanding"
                        ),
                    });
                }
            }
        }
    }
    risks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_usage_deltas_preserves_counters_and_last_use() {
        let event = |timestamp| UsageEvent {
            snippet_id: "example".into(),
            typed_chars: 2,
            inserted_chars: 5,
            unix_timestamp: timestamp,
        };
        let mut saved = UsageStats::default();
        saved.record(&event(1_700_000_000));
        let mut delta = UsageStats::default();
        delta.record(&event(1_700_000_001));
        delta.record(&event(1_700_000_002));

        saved.merge(&delta);

        let usage = &saved.snippets["example"];
        assert_eq!(usage.count, 3);
        assert_eq!(usage.chars_typed, 6);
        assert_eq!(usage.chars_inserted, 15);
        assert_eq!(usage.last_used, 1_700_000_002);
        assert_eq!(saved.daily.values().copied().sum::<u64>(), 3);
    }
}
