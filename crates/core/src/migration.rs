use crate::{
    config::{OrganizationPolicy, MAX_CONFIG_BYTES},
    Config, ConfigError, ExpansionConfig, MatchMode, Settings,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use thiserror::Error;

#[derive(Debug, Deserialize)]
struct EspansoDocument {
    #[serde(default)]
    matches: Vec<EspansoMatch>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
struct EspansoMatch {
    /// Espanso accepts either one `trigger` or a `triggers` list; extra
    /// triggers become aliases of a single WayExpand snippet.
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    triggers: Vec<String>,
    replace: Option<String>,
    label: Option<String>,
    #[serde(default)]
    propagate_case: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug)]
pub struct EspansoImport {
    pub config: Config,
    pub report: EspansoImportReport,
    /// Retained for existing callers; equals `report.unsupported`.
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EspansoImportMode {
    #[default]
    Permissive,
    Strict,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EspansoImportReport {
    pub fully_migrated: usize,
    pub migrated_with_warnings: usize,
    pub unsupported: usize,
    pub warnings: Vec<EspansoImportWarning>,
    pub unsupported_matches: Vec<EspansoUnsupportedMatch>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EspansoImportWarning {
    pub trigger: String,
    pub details: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EspansoUnsupportedMatch {
    pub trigger: String,
    pub reason: String,
}

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("could not read Espanso file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("Espanso file {path} is too large ({length} bytes; maximum is {maximum})")]
    TooLarge {
        path: String,
        length: usize,
        maximum: usize,
    },
    #[error("Espanso path is not a regular file: {path}")]
    NotRegular { path: String },
    #[error("could not parse Espanso YAML: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("imported configuration is invalid: {0}")]
    Invalid(#[from] ConfigError),
    #[error("strict Espanso import rejected {trigger}: {details}")]
    StrictRejected { trigger: String, details: String },
}

pub fn import_espanso(path: impl AsRef<Path>) -> Result<EspansoImport, MigrationError> {
    import_espanso_with_mode(path, EspansoImportMode::Permissive)
}

pub fn import_espanso_with_mode(
    path: impl AsRef<Path>,
    mode: EspansoImportMode,
) -> Result<EspansoImport, MigrationError> {
    let path = path.as_ref();
    let read_error = |source: std::io::Error| MigrationError::Read {
        path: path.display().to_string(),
        source,
    };
    let resolved_path = fs::canonicalize(path).map_err(read_error)?;
    // Open the resolved target nonblocking, then validate the descriptor. In
    // particular, opening a FIFO read-only without O_NONBLOCK could hang the
    // GUI before metadata or the bounded read is reached.
    let descriptor = rustix::fs::open(
        &resolved_path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| read_error(error.into()))?;
    let mut file = fs::File::from(descriptor);
    let metadata = file.metadata().map_err(read_error)?;
    if !metadata.file_type().is_file() {
        return Err(MigrationError::NotRegular {
            path: path.display().to_string(),
        });
    }
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(MigrationError::TooLarge {
            path: path.display().to_string(),
            length: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            maximum: MAX_CONFIG_BYTES,
        });
    }
    // Cap the read itself too: metadata can be stale or, for a non-regular
    // file, misleading about how many bytes are actually available.
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(MigrationError::TooLarge {
            path: path.display().to_string(),
            length: bytes.len(),
            maximum: MAX_CONFIG_BYTES,
        });
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        read_error(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.utf8_error(),
        ))
    })?;
    // The byte limit bounds the source document size; serde_yaml additionally
    // applies a recursion limit while deserializing nested structures.
    let document: EspansoDocument = serde_yaml::from_str(&text)?;
    let mut expansion = Vec::with_capacity(document.matches.len());
    let mut report = EspansoImportReport::default();
    for key in document.extra.keys() {
        if mode == EspansoImportMode::Strict {
            return Err(MigrationError::StrictRejected {
                trigger: "(file)".into(),
                details: format!("top-level option `{key}` is not imported"),
            });
        }
        report.warnings.push(EspansoImportWarning {
            trigger: "(file)".into(),
            details: vec![format!(
                "top-level option `{key}` is not imported; review its semantics"
            )],
        });
    }
    for item in document.matches {
        let mut triggers: Vec<String> = Vec::new();
        for trigger in item.trigger.into_iter().chain(item.triggers) {
            if !triggers.contains(&trigger) {
                triggers.push(trigger);
            }
        }
        if triggers.is_empty() {
            report.unsupported += 1;
            report.unsupported_matches.push(EspansoUnsupportedMatch {
                trigger: "(none)".into(),
                reason: "match has no `trigger` or `triggers` value".into(),
            });
            continue;
        }
        let trigger = triggers.remove(0);
        let aliases = triggers;
        let Some(replacement) = item.replace else {
            report.unsupported += 1;
            report.unsupported_matches.push(EspansoUnsupportedMatch {
                trigger,
                reason: "match has no static `replace` value (dynamic matches are unsupported)"
                    .into(),
            });
            continue;
        };
        let details = item
            .extra
            .keys()
            .map(|key| format!("option `{key}` is not mapped; review its semantics"))
            .collect::<Vec<_>>();
        if mode == EspansoImportMode::Strict && !details.is_empty() {
            report.unsupported += 1;
            report.unsupported_matches.push(EspansoUnsupportedMatch {
                trigger,
                reason: format!(
                    "strict mode discarded the match because {}",
                    details.join(", ")
                ),
            });
            continue;
        }
        if details.is_empty() {
            report.fully_migrated += 1;
        } else {
            report.migrated_with_warnings += 1;
            report.warnings.push(EspansoImportWarning {
                trigger: trigger.clone(),
                details,
            });
        }
        expansion.push(ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger,
            replacement,
            description: item.label.unwrap_or_default(),
            tags: vec!["imported".into()],
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: item.propagate_case,
            aliases,
        });
    }
    let config = Config {
        expansion,
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.validate()?;
    let skipped = report.unsupported;
    Ok(EspansoImport {
        config,
        report,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn imports_string_matches_and_counts_skipped_entries() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-espanso-{}.yml", std::process::id()));
        let mut file = fs::File::create(&path).unwrap();
        writeln!(
            file,
            "matches:\n  - trigger: ':hi'\n    replace: Hello\n    label: Greeting\n  - trigger: ':dynamic'"
        )
        .unwrap();
        let imported = import_espanso(&path).unwrap();
        assert_eq!(imported.config.expansion.len(), 1);
        assert_eq!(imported.config.expansion[0].trigger, ":hi");
        assert_eq!(imported.config.expansion[0].description, "Greeting");
        assert_eq!(imported.skipped, 1);
        assert_eq!(imported.report.fully_migrated, 1);
        assert_eq!(imported.report.unsupported, 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_semantics_that_are_not_mapped() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-espanso-report-{}.yml",
            std::process::id()
        ));
        fs::write(
            &path,
            "global_vars:\n  name: Example\nmatches:\n  - trigger: ':plain'\n    replace: Hello\n  - trigger: ':case'\n    replace: Hello\n    propagate_case: true\n  - trigger: ':word'\n    replace: Hello\n    word: true\n  - trigger: ':dynamic'\n    vars:\n      - name: output\n        type: shell\n",
        )
        .unwrap();

        let imported = import_espanso(&path).unwrap();
        assert_eq!(imported.report.fully_migrated, 2);
        assert_eq!(imported.report.migrated_with_warnings, 1);
        assert_eq!(imported.report.unsupported, 1);
        assert_eq!(imported.report.warnings.len(), 2); // also reports global_vars
        assert!(imported.config.expansion[1].propagate_case);
        assert!(!imported
            .report
            .warnings
            .iter()
            .any(|warning| warning.trigger == ":case"));
        assert!(imported.report.warnings.iter().any(|warning| {
            warning.trigger == ":word"
                && warning.details.iter().any(|detail| detail.contains("word"))
        }));
        assert_eq!(imported.report.unsupported_matches[0].trigger, ":dynamic");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn strict_mode_discards_semantically_changed_matches_and_reports_them() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-espanso-strict-{}.yml",
            std::process::id()
        ));
        fs::write(
            &path,
            "matches:\n  - trigger: ':plain'\n    replace: Hello\n  - trigger: ':word'\n    replace: Hello\n    word: true\n",
        )
        .unwrap();

        let imported = import_espanso_with_mode(&path, EspansoImportMode::Strict).unwrap();
        assert_eq!(imported.config.expansion.len(), 1);
        assert_eq!(imported.report.fully_migrated, 1);
        assert_eq!(imported.report.unsupported, 1);
        assert_eq!(imported.report.unsupported_matches[0].trigger, ":word");
        assert!(imported.report.unsupported_matches[0]
            .reason
            .contains("word"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_fifo_without_waiting_for_a_writer() {
        let path =
            std::env::temp_dir().join(format!("wayexpand-espanso-fifo-{}.yml", std::process::id()));
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &path,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        assert!(matches!(
            import_espanso(&path),
            Err(MigrationError::NotRegular { .. })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn oversized_espanso_file_is_rejected_before_parsing() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-espanso-oversized-{}.yml",
            std::process::id()
        ));
        let mut file = fs::File::create(&path).unwrap();
        // Content doesn't need to be valid YAML: the size check runs first.
        file.write_all(&vec![b'a'; MAX_CONFIG_BYTES + 1]).unwrap();
        assert!(matches!(
            import_espanso(&path),
            Err(MigrationError::TooLarge { .. })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn triggers_lists_become_one_snippet_with_aliases() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-espanso-triggers-{}.yml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "matches:\n  - triggers: [':addr', ':address', ':office']\n    replace: 1 Main St\n  - trigger: ':hi'\n    triggers: [':hi', ':hello']\n    replace: Hello\n",
        )
        .unwrap();
        let import = import_espanso(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(import.config.expansion.len(), 2);
        assert_eq!(import.config.expansion[0].trigger, ":addr");
        assert_eq!(import.config.expansion[0].aliases, [":address", ":office"]);
        assert_eq!(import.config.expansion[1].trigger, ":hi");
        assert_eq!(import.config.expansion[1].aliases, [":hello"]);
        assert_eq!(import.report.fully_migrated, 2);
    }
}
