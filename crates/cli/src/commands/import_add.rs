//! Merge imported expansions into a library after exercising their triggers
//! and rendering paths through the same core matcher used by the daemon.

use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use wayexpand_core::{Config, ExpansionConfig, ExpansionEngine, InputEvent};

#[derive(Debug, Default)]
pub(crate) struct ImportAddReport {
    pub(crate) added: usize,
    pub(crate) identical_duplicates: usize,
    pub(crate) conflicts_kept: usize,
    pub(crate) expansions_tested: usize,
    pub(crate) trigger_checks: usize,
    pub(crate) form_checks: usize,
    pub(crate) command_checks_not_executed: usize,
    pub(crate) app_filters_not_exercised: usize,
    pub(crate) backup_path: Option<String>,
    pub(crate) daemon_reload_confirmed: Option<bool>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MergeStats {
    added: usize,
    identical_duplicates: usize,
    conflicts_kept: usize,
}

/// Add imported expansions to `path`, preserving existing settings and
/// expansions. Every changed expansion is validated and smoke-tested before
/// an atomic, revision-checked write; the old file is backed up first.
pub(crate) fn add_to_library(path: &Path, imported: &Config) -> Result<ImportAddReport> {
    let current = Config::load_versioned(path)
        .with_context(|| format!("could not load destination library {}", path.display()))?;
    let (merged, stats, added_expansions) = merge_imported(&current.config, imported);
    merged
        .validate()
        .context("merged import failed configuration validation")?;

    let smoke = smoke_test_imported(&merged, &added_expansions)?;
    let mut report = ImportAddReport {
        added: stats.added,
        identical_duplicates: stats.identical_duplicates,
        conflicts_kept: stats.conflicts_kept,
        expansions_tested: smoke.expansions_tested,
        trigger_checks: smoke.trigger_checks,
        form_checks: smoke.form_checks,
        command_checks_not_executed: smoke.command_checks_not_executed,
        app_filters_not_exercised: smoke.app_filters_not_exercised,
        backup_path: None,
        daemon_reload_confirmed: None,
    };

    if stats.added == 0 {
        return Ok(report);
    }

    // Validate and test before creating a backup or changing the live config.
    let backup = crate::backup::default_backup_destination(path);
    crate::backup::create_backup(path, &backup)
        .with_context(|| format!("could not back up destination library {}", path.display()))?;
    merged
        .save_atomic_if_revision_matches(path, &current.revision)
        .with_context(|| {
            format!(
                "destination library changed during import; the previous file is backed up at {}",
                backup.display()
            )
        })?;
    report.backup_path = Some(backup.display().to_string());
    if same_path(path, &wayexpand_core::default_config_path()) {
        report.daemon_reload_confirmed = Some(
            wayexpand_core::DaemonClient::from_environment()
                .and_then(|client| client.execute(wayexpand_core::DaemonOperation::Reload))
                .is_ok(),
        );
    }
    Ok(report)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

fn merge_imported(
    current: &Config,
    imported: &Config,
) -> (Config, MergeStats, Vec<ExpansionConfig>) {
    let mut merged = current.clone();
    let mut stats = MergeStats::default();
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

    let mut added = Vec::new();
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
            Some(existing) if same_expansion_content(existing, candidate) => {
                stats.identical_duplicates += 1;
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
                added.push(candidate.clone());
                stats.added += 1;
            }
        }
    }
    (merged, stats, added)
}

fn same_expansion_content(left: &ExpansionConfig, right: &ExpansionConfig) -> bool {
    let mut left_without_identity = left.clone();
    left_without_identity.id = right.id.clone();
    left_without_identity == *right
}

#[derive(Debug, Default)]
struct SmokeTestStats {
    expansions_tested: usize,
    trigger_checks: usize,
    form_checks: usize,
    command_checks_not_executed: usize,
    app_filters_not_exercised: usize,
}

fn smoke_test_imported(merged: &Config, imported: &[ExpansionConfig]) -> Result<SmokeTestStats> {
    let imported_ids = imported
        .iter()
        .map(|expansion| expansion.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut smoke_config = merged.clone();
    let mut app_filters_not_exercised = 0;
    for expansion in &mut smoke_config.expansion {
        // The core engine can dispatch command-backed expansions. Remove
        // commands from this throwaway copy so no existing or imported
        // command can run during verification; persisted config is untouched.
        expansion.command = None;
        if imported_ids.contains(expansion.id.as_str()) {
            if !expansion.app_filter.is_empty() {
                app_filters_not_exercised += 1;
                // A generic CLI process has no focused-window identity. Test
                // matching and rendering independently; the imported filter
                // itself remains validated and unchanged in `merged`.
                expansion.app_filter.clear();
            }
            // Disabled imports are tested in an enabled smoke-test copy so
            // we verify their trigger and renderer without changing the
            // disabled state that will be saved to the library.
            expansion.enabled = true;
        }
    }
    smoke_config.organization = Default::default();
    smoke_config
        .validate()
        .context("import smoke-test config is invalid")?;
    let mut engine = ExpansionEngine::new(smoke_config)
        .context("could not initialize imported expansions for smoke tests")?;
    let mut stats = SmokeTestStats {
        app_filters_not_exercised,
        ..SmokeTestStats::default()
    };

    for expansion in imported {
        for trigger in expansion.effective_triggers() {
            let mut results = engine.process(InputEvent::Text(trigger.clone()));
            results.extend(engine.process(InputEvent::EndOfInput));
            let matched = results
                .iter()
                .find(|result| result.snippet_id.as_ref() == expansion.id);
            if matched.is_none() {
                let is_form = wayexpand_core::form_fields(&expansion.replacement)
                    .is_ok_and(|fields| !fields.is_empty());
                let explanation = engine.explain(&trigger, "import-smoke-test");
                let recognized_form = is_form
                    && explanation.snippet.as_deref() == Some(expansion.trigger.as_str())
                    && explanation.checks.iter().any(|check| {
                        check.name == "trigger" && check.status == wayexpand_core::CheckStatus::Pass
                    });
                if recognized_form {
                    stats.form_checks += 1;
                } else {
                    engine.process(InputEvent::Reset);
                    bail!(
                        "import smoke test failed for expansion {}: its trigger did not produce a match",
                        expansion.id
                    );
                }
            } else if expansion.command.is_some() {
                // Its trigger and static text were exercised, but the
                // original command was removed from the smoke-test copy.
                stats.command_checks_not_executed += 1;
            }
            stats.trigger_checks += 1;
            engine.process(InputEvent::Reset);
        }
        stats.expansions_tested += 1;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config(text: &str) -> Config {
        Config::parse(text).unwrap()
    }

    #[test]
    fn merge_preserves_existing_config_and_skips_conflicts() {
        let current = config(
            "[[expansion]]\ntrigger=';existing'\nreplacement='keep'\n\n[[expansion]]\ntrigger=';conflict'\nreplacement='old'\n\n[[expansion]]\ntrigger=';identical'\nreplacement='same'\n",
        );
        let imported = config(
            "[[expansion]]\ntrigger=';new'\nreplacement='new text'\n\n[[expansion]]\ntrigger=';conflict'\nreplacement='new text'\n\n[[expansion]]\ntrigger=';identical'\nreplacement='same'\n",
        );
        let (merged, stats, added) = merge_imported(&current, &imported);
        assert_eq!(stats.added, 1);
        assert_eq!(stats.conflicts_kept, 1);
        assert_eq!(stats.identical_duplicates, 1);
        assert_eq!(merged.expansion.len(), 4);
        assert_eq!(merged.expansion[1].replacement, "old");
        assert_eq!(added.len(), 1);
        merged.validate().unwrap();
    }

    #[test]
    fn smoke_tests_primary_triggers_aliases_and_templates_without_running_commands() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-import-command-marker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!marker.exists());
        let imported = config(
            &format!("[[expansion]]\ntrigger=';hello'\naliases=[';hi']\nreplacement='Hello {{{{cursor}}}}'\n\n[[expansion]]\ntrigger=';cmd'\nreplacement='safe text'\ncommand={{program='/usr/bin/touch',args=['{}']}}\n", marker.display()),
        );
        let empty = config("");
        let (merged, _, added) = merge_imported(&empty, &imported);
        let report = smoke_test_imported(&merged, &added).unwrap();
        assert_eq!(report.trigger_checks, 3);
        assert_eq!(report.expansions_tested, 2);
        assert_eq!(report.command_checks_not_executed, 1);
        assert_eq!(report.form_checks, 0);
        assert!(
            !marker.exists(),
            "smoke testing executed an imported command"
        );
    }

    #[test]
    fn smoke_test_reports_forms_as_recognized_without_collecting_values() {
        let imported =
            config("[[expansion]]\ntrigger=';form'\nreplacement='Hello {{field:name}}'\n");
        let empty = config("");
        let (merged, _, added) = merge_imported(&empty, &imported);
        let report = smoke_test_imported(&merged, &added).unwrap();
        assert_eq!(report.form_checks, 1);
        assert_eq!(report.trigger_checks, 1);
        assert_eq!(report.expansions_tested, 1);
    }

    #[test]
    fn add_to_library_tests_then_saves_and_backs_up_atomically() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-import-add-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("expansions.toml");
        let current = config("[[expansion]]\ntrigger=';old'\nreplacement='old text'\n");
        Config::save_atomic_text(&path, &toml::to_string_pretty(&current).unwrap()).unwrap();
        let imported = config("[[expansion]]\ntrigger=';new'\nreplacement='new text'\n");

        let report = add_to_library(&path, &imported).unwrap();
        let saved = Config::load(&path).unwrap();
        assert_eq!(report.added, 1);
        assert_eq!(report.trigger_checks, 1);
        assert_eq!(report.expansions_tested, 1);
        assert!(report
            .backup_path
            .as_ref()
            .is_some_and(|backup| Path::new(backup).is_file()));
        assert_eq!(saved.expansion.len(), 2);
        assert!(saved
            .expansion
            .iter()
            .any(|expansion| expansion.trigger == ";old"));
        assert!(saved
            .expansion
            .iter()
            .any(|expansion| expansion.trigger == ";new"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
