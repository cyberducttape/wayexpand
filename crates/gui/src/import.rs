use std::path::Path;

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
    for candidate in &imported.expansion {
        let candidate_triggers = candidate.effective_triggers();
        match merged.expansion.iter().find(|existing| {
            existing.id == candidate.id
                || existing.trigger == candidate.trigger
                || (existing.enabled
                    && candidate.enabled
                    && existing
                        .effective_triggers()
                        .iter()
                        .any(|trigger| candidate_triggers.contains(trigger)))
        }) {
            Some(existing) if expansion_content_equal(existing, candidate) => {
                stats.identical_duplicates += 1
            }
            Some(_) => stats.conflicts_kept += 1,
            None => {
                merged.expansion.push(candidate.clone());
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
