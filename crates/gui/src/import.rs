use std::{collections::HashMap, path::Path};

use anyhow::Result;
use wayexpand_core::{import_espanso, Config, EspansoImportReport, ExpansionConfig};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ImportMergeStats {
    pub(crate) added: usize,
    pub(crate) identical_duplicates: usize,
    pub(crate) conflicts_kept: usize,
}

/// Merge imported snippets without replacing the user's existing library.
/// Exact duplicates are ignored and trigger collisions are kept unchanged.
pub(crate) fn merge_imported_expansions(
    current: &Config,
    imported: &Config,
) -> (Config, ImportMergeStats) {
    let mut merged = current.clone();
    let mut stats = ImportMergeStats::default();
    // Keep the first matching position for each identity/trigger. Selecting
    // the lowest matching index below preserves the old vector-scan
    // semantics while avoiding an O(imported * existing) merge.
    let mut by_id = HashMap::new();
    let mut by_trigger = HashMap::new();
    let mut by_effective_trigger = HashMap::new();
    for (index, existing) in merged.expansion.iter().enumerate() {
        by_id.entry(existing.id.clone()).or_insert(index);
        by_trigger.entry(existing.trigger.clone()).or_insert(index);
        if existing.enabled {
            for trigger in existing.effective_triggers() {
                by_effective_trigger.entry(trigger).or_insert(index);
            }
        }
    }
    for candidate in &imported.expansion {
        let candidate_triggers = candidate.effective_triggers();
        let mut matching_index: Option<usize> = None;
        let mut consider = |index| {
            matching_index = Some(matching_index.map_or(index, |current| current.min(index)));
        };
        if let Some(&index) = by_id.get(&candidate.id) {
            consider(index);
        }
        if let Some(&index) = by_trigger.get(&candidate.trigger) {
            consider(index);
        }
        if candidate.enabled {
            for trigger in &candidate_triggers {
                if let Some(&index) = by_effective_trigger.get(trigger) {
                    consider(index);
                }
            }
        }
        match matching_index.and_then(|index| merged.expansion.get(index)) {
            Some(existing) if expansion_content_equal(existing, candidate) => {
                stats.identical_duplicates += 1
            }
            Some(_) => stats.conflicts_kept += 1,
            None => {
                let index = merged.expansion.len();
                merged.expansion.push(candidate.clone());
                by_id.entry(candidate.id.clone()).or_insert(index);
                by_trigger.entry(candidate.trigger.clone()).or_insert(index);
                if candidate.enabled {
                    for trigger in candidate_triggers {
                        by_effective_trigger.entry(trigger).or_insert(index);
                    }
                }
                stats.added += 1;
            }
        }
    }
    (merged, stats)
}

fn expansion_content_equal(left: &ExpansionConfig, right: &ExpansionConfig) -> bool {
    let mut left_without_identity = left.clone();
    left_without_identity.id = right.id.clone();
    left_without_identity == *right
}

/// Load and validate an Espanso library for review without mutating the
/// active configuration. The caller decides when to apply the preview.
pub(crate) fn preview_espanso(path: &Path) -> Result<(Config, EspansoImportReport)> {
    let imported = import_espanso(path)?;
    Ok((imported.config, imported.report))
}
