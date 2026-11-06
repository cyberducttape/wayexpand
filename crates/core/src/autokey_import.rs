//! Safe import of static AutoKey phrase files.
//!
//! AutoKey stores a phrase in a UTF-8 `.txt` file and its triggers/settings in
//! a JSON sidecar named `.<phrase>.json`. Python scripts and executable
//! macros are intentionally never run during import.

use crate::{
    config::{
        OrganizationPolicy, MAX_CONFIG_BYTES, MAX_EFFECTIVE_TRIGGERS,
        MAX_EFFECTIVE_TRIGGER_SCALARS, MAX_TOTAL_TRIGGER_CHARS,
    },
    Config, ConfigError, ExpansionConfig, MatchMode, Settings,
};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use thiserror::Error;

const MAX_PHRASES: usize = 4096;
const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_METADATA_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AutoKeyImportMode {
    #[default]
    Permissive,
    Strict,
}

#[derive(Debug)]
pub struct AutoKeyImport {
    pub config: Config,
    pub report: AutoKeyImportReport,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct AutoKeyImportReport {
    pub imported: usize,
    /// Entries that could not be imported. Retained separately from
    /// `unsupported` so reports describe I/O and malformed metadata too.
    pub skipped: usize,
    /// Skipped entries that need an operator to review or repair source
    /// behavior before retrying migration.
    pub needs_review: usize,
    /// Legacy count of entries skipped because their AutoKey behavior cannot
    /// be represented by WayExpand.
    pub unsupported: usize,
    pub warnings: Vec<AutoKeyImportWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct AutoKeyImportWarning {
    pub entry: String,
    pub reason: String,
    pub remediation: String,
}

#[derive(Debug, Error)]
pub enum AutoKeyImportError {
    #[error("could not access AutoKey path {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("AutoKey path is not a real directory or regular file: {0}")]
    UnsafePath(String),
    #[error("AutoKey import exceeds the {maximum}-byte source limit")]
    TooLarge { maximum: usize },
    #[error("AutoKey import exceeds the {MAX_PHRASES}-phrase limit")]
    TooManyPhrases,
    #[error("could not parse AutoKey phrase metadata {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("duplicate AutoKey abbreviation {trigger:?} in {path}")]
    DuplicateTrigger { path: String, trigger: String },
    #[error("strict AutoKey import rejected {entry}: {reason}")]
    StrictRejected { entry: String, reason: String },
    #[error("imported AutoKey configuration is invalid: {0}")]
    Invalid(#[from] ConfigError),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PhraseMetadata {
    #[serde(rename = "type")]
    kind: String,
    description: String,
    abbreviation: Option<AbbreviationMetadata>,
    hotkey: Option<HotkeyMetadata>,
    filter: Option<FilterMetadata>,
    #[serde(rename = "omitTrigger")]
    omit_trigger: bool,
    prompt: bool,
    #[serde(rename = "matchCase")]
    match_case: bool,
    modes: Option<Vec<u8>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AbbreviationMetadata {
    abbreviations: Vec<String>,
    #[serde(rename = "wordChars")]
    word_chars: Option<String>,
    immediate: bool,
    #[serde(rename = "ignoreCase")]
    ignore_case: bool,
    backspace: bool,
    #[serde(rename = "triggerInside")]
    trigger_inside: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HotkeyMetadata {
    #[serde(rename = "hotKey")]
    hot_key: Option<String>,
    modifiers: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FilterMetadata {
    regex: Option<String>,
}

/// Import a single AutoKey phrase pair or a directory tree of phrase pairs.
/// Directory symlinks are not followed. If `path` is a `.txt` phrase file,
/// its sibling hidden JSON sidecar is required.
pub fn import_autokey(
    path: impl AsRef<Path>,
    mode: AutoKeyImportMode,
) -> Result<AutoKeyImport, AutoKeyImportError> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(AutoKeyImportError::UnsafePath(path.display().to_string()));
    }
    let mut phrases = Vec::new();
    let mut report = AutoKeyImportReport::default();
    if metadata.is_dir() {
        collect_phrase_files(path, path, 0, &mut phrases, mode, &mut report)?;
    } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "txt") {
        phrases.push(path.to_path_buf());
    } else {
        return Err(AutoKeyImportError::UnsafePath(path.display().to_string()));
    }
    phrases.sort();
    if phrases.is_empty() && report.skipped == 0 {
        return Err(AutoKeyImportError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no AutoKey .txt phrase files found",
            ),
        });
    }

    let mut total_bytes = 0usize;
    let mut config = Config {
        expansion: Vec::new(),
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let mut triggers = HashSet::new();
    let mut trigger_chars = 0usize;
    let mut effective_trigger_chars = 0usize;
    let mut effective_trigger_count = 0usize;
    for phrase_path in phrases {
        let relative = if phrase_path == path {
            phrase_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            phrase_path
                .strip_prefix(path)
                .unwrap_or(&phrase_path)
                .to_string_lossy()
                .into_owned()
        };
        let phrase = match read_bounded_file(&phrase_path, MAX_CONFIG_BYTES) {
            Ok(phrase) => phrase,
            Err(error) => {
                if mode == AutoKeyImportMode::Strict {
                    return Err(error);
                }
                report.skipped += 1;
                report.needs_review += 1;
                report.warnings.push(AutoKeyImportWarning {
                    entry: relative,
                    reason: error.to_string(),
                    remediation: "Restore or repair this phrase file, then retry the import."
                        .into(),
                });
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(phrase.len());
        if total_bytes > MAX_CONFIG_BYTES {
            return Err(AutoKeyImportError::TooLarge {
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let sidecar = metadata_path(&phrase_path);
        let metadata_bytes = match read_bounded_file(&sidecar, MAX_METADATA_BYTES) {
            Ok(metadata) => metadata,
            Err(error) => {
                if mode == AutoKeyImportMode::Strict {
                    return Err(error);
                }
                report.skipped += 1;
                report.needs_review += 1;
                report.warnings.push(AutoKeyImportWarning {
                    entry: sidecar
                        .strip_prefix(path)
                        .unwrap_or(&sidecar)
                        .to_string_lossy()
                        .into_owned(),
                    reason: error.to_string(),
                    remediation: "Restore the matching AutoKey JSON sidecar or remove the orphaned phrase file."
                        .into(),
                });
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(metadata_bytes.len());
        if total_bytes > MAX_CONFIG_BYTES {
            return Err(AutoKeyImportError::TooLarge {
                maximum: MAX_CONFIG_BYTES,
            });
        }
        let metadata: PhraseMetadata = match serde_json::from_str(&metadata_bytes) {
            Ok(metadata) => metadata,
            Err(source) => {
                let error = AutoKeyImportError::Metadata {
                    path: sidecar.display().to_string(),
                    source,
                };
                if mode == AutoKeyImportMode::Strict {
                    return Err(error);
                }
                report.skipped += 1;
                report.needs_review += 1;
                report.warnings.push(AutoKeyImportWarning {
                    entry: sidecar
                        .strip_prefix(path)
                        .unwrap_or(&sidecar)
                        .to_string_lossy()
                        .into_owned(),
                    reason: error.to_string(),
                    remediation: "Fix the sidecar JSON syntax and retry the import.".into(),
                });
                continue;
            }
        };
        match convert_phrase(&relative, phrase, &metadata, mode, &mut report)? {
            Some(expansion) => {
                // Validate each phrase before adding it to the aggregate. In
                // permissive mode, one invalid trigger/template/field must
                // not discard otherwise importable phrases when the final
                // Config::validate() runs.
                let candidate = Config {
                    expansion: vec![expansion.clone()],
                    hotkey: Vec::new(),
                    settings: config.settings.clone(),
                    organization: config.organization.clone(),
                };
                let candidate_validation = candidate.validate();
                if let Err(error) = candidate_validation {
                    if mode == AutoKeyImportMode::Strict {
                        return Err(AutoKeyImportError::StrictRejected {
                            entry: relative,
                            reason: error.to_string(),
                        });
                    }
                    report.skipped += 1;
                    report.needs_review += 1;
                    report.warnings.push(AutoKeyImportWarning {
                        entry: relative,
                        reason: error.to_string(),
                        remediation: "Repair the phrase trigger or replacement so it satisfies WayExpand configuration validation, then retry the import.".into(),
                    });
                    continue;
                }
                // Keep aggregate limits and effective-trigger collisions
                // aligned with Config::validate without repeatedly cloning
                // the entire growing import for every phrase.
                let configured_trigger_chars = std::iter::once(&expansion.trigger)
                    .chain(&expansion.aliases)
                    .map(|trigger| trigger.chars().count())
                    .sum::<usize>();
                let effective_triggers = expansion.effective_triggers();
                let next_trigger_chars = trigger_chars.saturating_add(configured_trigger_chars);
                let next_effective_chars = effective_trigger_chars.saturating_add(
                    effective_triggers
                        .iter()
                        .map(|trigger| trigger.chars().count())
                        .sum::<usize>(),
                );
                let next_effective_count =
                    effective_trigger_count.saturating_add(effective_triggers.len());
                let duplicate = effective_triggers
                    .iter()
                    .find(|trigger| triggers.contains(*trigger));
                let aggregate_limit_exceeded = next_trigger_chars > MAX_TOTAL_TRIGGER_CHARS
                    || next_effective_chars > MAX_EFFECTIVE_TRIGGER_SCALARS
                    || next_effective_count > MAX_EFFECTIVE_TRIGGERS;
                if duplicate.is_some() || aggregate_limit_exceeded {
                    let reason = duplicate.map_or_else(
                        || {
                            "adding this phrase would exceed WayExpand's aggregate trigger limits"
                                .to_owned()
                        },
                        |trigger| {
                            format!("duplicate AutoKey abbreviation {trigger:?} (effective trigger collision)")
                        },
                    );
                    if mode == AutoKeyImportMode::Strict {
                        if let Some(trigger) = duplicate {
                            return Err(AutoKeyImportError::DuplicateTrigger {
                                path: relative,
                                trigger: trigger.clone(),
                            });
                        }
                        return Err(AutoKeyImportError::StrictRejected {
                            entry: relative,
                            reason,
                        });
                    }
                    report.unsupported += usize::from(duplicate.is_some());
                    report.skipped += 1;
                    report.needs_review += 1;
                    report.warnings.push(AutoKeyImportWarning {
                        entry: relative,
                        reason,
                        remediation: "Rename the conflicting abbreviation or reduce the imported trigger set, then retry.".into(),
                    });
                    continue;
                }
                trigger_chars = next_trigger_chars;
                effective_trigger_chars = next_effective_chars;
                effective_trigger_count = next_effective_count;
                triggers.extend(effective_triggers);
                config.expansion.push(expansion);
                report.imported += 1;
            }
            None => {
                report.unsupported += 1;
                report.skipped += 1;
                report.needs_review += 1;
            }
        }
    }
    config.validate()?;
    Ok(AutoKeyImport { config, report })
}

fn convert_phrase(
    entry: &str,
    mut phrase: String,
    metadata: &PhraseMetadata,
    mode: AutoKeyImportMode,
    report: &mut AutoKeyImportReport,
) -> Result<Option<ExpansionConfig>, AutoKeyImportError> {
    let Some(abbreviation) = metadata.abbreviation.as_ref() else {
        return unsupported(entry, "phrase has no abbreviation metadata", mode, report);
    };
    let hotkey_is_active = metadata.hotkey.as_ref().is_some_and(|hotkey| {
        hotkey.hot_key.as_ref().is_some_and(|key| !key.is_empty()) || !hotkey.modifiers.is_empty()
    });
    let filter_is_active = metadata
        .filter
        .as_ref()
        .and_then(|filter| filter.regex.as_deref())
        .is_some_and(|regex| !regex.is_empty());
    let reasons = [
        (metadata.kind != "phrase", "entry is not a static phrase"),
        (
            metadata.modes.as_deref() != Some(&[1]),
            "phrase trigger modes are missing or include unsupported modes",
        ),
        (
            abbreviation.abbreviations.is_empty(),
            "phrase has no abbreviations",
        ),
        (
            metadata.omit_trigger,
            "omitTrigger behavior is not supported",
        ),
        (metadata.prompt, "prompted phrase behavior is not supported"),
        (
            hotkey_is_active,
            "phrase also has a hotkey, which is not imported",
        ),
        (
            filter_is_active,
            "window filters cannot be represented safely",
        ),
        (
            abbreviation.ignore_case,
            "case-insensitive matching is not supported",
        ),
        (
            abbreviation.trigger_inside,
            "inside-word matching is not supported",
        ),
        (
            abbreviation
                .word_chars
                .as_deref()
                .is_some_and(|chars| chars != r"[\w]"),
            "custom abbreviation boundary characters are not supported",
        ),
        (
            !abbreviation.backspace,
            "non-erasing abbreviation behavior is not supported",
        ),
    ];
    if let Some((_, reason)) = reasons.into_iter().find(|(unsupported, _)| *unsupported) {
        return unsupported(entry, reason, mode, report);
    }

    // AutoKey's cursor marker is a non-executable phrase feature with direct
    // WayExpand semantics. Other known dynamic macros may read files, run
    // scripts, or execute commands, so preserve no behavior that would
    // silently broaden the imported snippet's authority.
    // AutoKey phrases are plain text except for AutoKey's own macros. Escape
    // WayExpand's template delimiters so imported infrastructure text such as
    // `{{ticket.requester.name}}` remains literal and cannot be evaluated as
    // an unintended WayExpand variable.
    phrase = phrase.replace("{{", "{{{{").replace("}}", "}}}}");
    phrase = phrase.replace("<cursor>", "{{cursor}}");
    for macro_name in ["date", "file", "script", "system"] {
        if contains_autokey_macro(&phrase, macro_name) {
            return unsupported(
                entry,
                "phrase contains a dynamic AutoKey macro; convert it explicitly after import",
                mode,
                report,
            );
        }
    }
    let mut abbreviations = abbreviation.abbreviations.iter();
    let trigger = abbreviations
        .next()
        .expect("empty abbreviations were rejected above")
        .clone();
    let aliases = abbreviations.cloned().collect();
    Ok(Some(ExpansionConfig {
        id: ExpansionConfig::new_id(),
        trigger,
        replacement: phrase,
        description: metadata.description.clone(),
        tags: Vec::new(),
        category: Path::new(entry)
            .parent()
            .filter(|parent| *parent != Path::new("."))
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default(),
        app_filter: Vec::new(),
        match_mode: if abbreviation.immediate {
            MatchMode::Immediate
        } else {
            MatchMode::WordBoundary
        },
        command: None,
        enabled: true,
        propagate_case: metadata.match_case,
        aliases,
        output_file: None,
    }))
}

fn contains_autokey_macro(phrase: &str, name: &str) -> bool {
    phrase
        .match_indices(&format!("<{name}"))
        .any(|(start, matched)| {
            phrase[start + matched.len()..]
                .chars()
                .next()
                .is_none_or(|character| character.is_whitespace() || character == '>')
        })
}

fn unsupported(
    entry: &str,
    reason: &str,
    mode: AutoKeyImportMode,
    report: &mut AutoKeyImportReport,
) -> Result<Option<ExpansionConfig>, AutoKeyImportError> {
    if mode == AutoKeyImportMode::Strict {
        return Err(AutoKeyImportError::StrictRejected {
            entry: entry.to_owned(),
            reason: reason.to_owned(),
        });
    }
    report.warnings.push(AutoKeyImportWarning {
        entry: entry.to_owned(),
        reason: reason.to_owned(),
        remediation: "Review this entry and convert its unsupported AutoKey behavior manually."
            .into(),
    });
    report.needs_review += 1;
    Ok(None)
}

fn collect_phrase_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    phrases: &mut Vec<PathBuf>,
    mode: AutoKeyImportMode,
    report: &mut AutoKeyImportReport,
) -> Result<(), AutoKeyImportError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(AutoKeyImportError::TooManyPhrases);
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(source) if depth == 0 => return Err(io_error(directory, source)),
        Err(source) => {
            record_traversal_error(
                root,
                directory,
                source,
                mode,
                report,
                "Restore read access to this folder, then retry the import. Accessible sibling entries were retained.",
            )?;
            return Ok(());
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                record_traversal_error(
                    root,
                    directory,
                    source,
                    mode,
                    report,
                    "Restore directory access and retry the import. Entries already read were retained.",
                )?;
                continue;
            }
        };
        let path = entry.path();
        // macOS may add AppleDouble resource-fork files when a folder is
        // copied to a filesystem that cannot store those forks. They are
        // metadata, not AutoKey phrases, even when the original has a .txt
        // suffix (for example, `._greeting.txt`).
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("._"))
        {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => {
                record_traversal_error(
                    root,
                    &path,
                    source,
                    mode,
                    report,
                    "Restore or repair this entry, then retry the import.",
                )?;
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_phrase_files(root, &path, depth + 1, phrases, mode, report)?;
        } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "txt")
        {
            phrases.push(path);
            if phrases.len() > MAX_PHRASES {
                return Err(AutoKeyImportError::TooManyPhrases);
            }
        }
    }
    Ok(())
}

fn record_traversal_error(
    root: &Path,
    path: &Path,
    source: std::io::Error,
    mode: AutoKeyImportMode,
    report: &mut AutoKeyImportReport,
    remediation: &str,
) -> Result<(), AutoKeyImportError> {
    if mode == AutoKeyImportMode::Strict {
        return Err(io_error(path, source));
    }
    let entry = path
        .strip_prefix(root)
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                Path::new(".")
            } else {
                relative
            }
        })
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    report.skipped += 1;
    report.needs_review += 1;
    report.warnings.push(AutoKeyImportWarning {
        entry,
        reason: io_error(path, source).to_string(),
        remediation: remediation.into(),
    });
    Ok(())
}

fn metadata_path(phrase: &Path) -> PathBuf {
    let stem = phrase.file_stem().unwrap_or_default().to_string_lossy();
    let parent = phrase.parent().unwrap_or_else(|| Path::new("."));
    let hidden_sidecar = parent.join(format!(".{stem}.json"));
    // AutoKey data sets commonly use a JSON file with the same basename as
    // the phrase (`greeting.txt` + `greeting.json`). Keep accepting the hidden
    // sidecar form as well, which is used by other exports and older imports.
    match fs::symlink_metadata(&hidden_sidecar) {
        Ok(_) => hidden_sidecar,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            parent.join(format!("{stem}.json"))
        }
        Err(_) => hidden_sidecar,
    }
}

fn read_bounded_file(path: &Path, maximum: usize) -> Result<String, AutoKeyImportError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AutoKeyImportError::UnsafePath(path.display().to_string()));
    }
    if metadata.len() > maximum as u64 {
        return Err(AutoKeyImportError::TooLarge { maximum });
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !opened.is_file() || opened.len() > maximum as u64 {
        return Err(AutoKeyImportError::UnsafePath(path.display().to_string()));
    }
    let mut contents = String::new();
    file.take(maximum as u64 + 1)
        .read_to_string(&mut contents)
        .map_err(|source| io_error(path, source))?;
    if contents.len() > maximum {
        return Err(AutoKeyImportError::TooLarge { maximum });
    }
    Ok(contents)
}

fn io_error(path: &Path, source: std::io::Error) -> AutoKeyImportError {
    AutoKeyImportError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "wayexpand-autokey-import-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn phrase(&self, name: &str, text: &str, metadata: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, text).unwrap();
            let stem = Path::new(name).file_stem().unwrap().to_string_lossy();
            fs::write(path.with_file_name(format!(".{stem}.json")), metadata).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn metadata(abbreviations: &[&str]) -> String {
        serde_json::json!({
            "type": "phrase",
            "modes": [1],
            "description": "Imported phrase",
            "matchCase": true,
            "abbreviation": {
                "abbreviations": abbreviations,
                "wordChars": r"[\w]",
                "immediate": false,
                "ignoreCase": false,
                "backspace": true,
                "triggerInside": false
            },
            "hotkey": {"hotKey": null, "modifiers": []},
            "filter": null
        })
        .to_string()
    }

    #[test]
    fn imports_phrase_text_aliases_cursor_and_folder_category() {
        let directory = TestDirectory::new();
        let folder = directory.0.join("Support");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("signature.txt"), "Regards, <cursor>Sam").unwrap();
        fs::write(
            folder.join(".signature.json"),
            metadata(&[";sig", ";signature"]),
        )
        .unwrap();

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Strict).unwrap();
        assert_eq!(imported.report.imported, 1);
        let expansion = &imported.config.expansion[0];
        assert_eq!(expansion.trigger, ";sig");
        assert_eq!(expansion.aliases, [";signature"]);
        assert_eq!(expansion.replacement, "Regards, {{cursor}}Sam");
        assert_eq!(expansion.category, "Support");
        assert_eq!(expansion.match_mode, MatchMode::WordBoundary);
        assert!(expansion.propagate_case);
    }

    #[test]
    fn permissive_import_skips_unreadable_nested_directory_and_continues() {
        let directory = TestDirectory::new();
        directory.phrase("available.txt", "Kept", &metadata(&[";available"]));
        let restricted = directory.0.join("000-private");
        fs::create_dir(&restricted).unwrap();
        fs::write(restricted.join("hidden.txt"), "not readable").unwrap();
        fs::write(restricted.join(".hidden.json"), metadata(&[";hidden"])).unwrap();
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0)).unwrap();

        // Root-run test environments bypass Unix directory read permissions;
        // hosted CI and ordinary desktop users exercise the denied traversal.
        let permissions_are_enforced = fs::read_dir(&restricted).is_err();
        if !permissions_are_enforced {
            fs::set_permissions(&restricted, fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }

        let strict = import_autokey(&directory.0, AutoKeyImportMode::Strict);
        let permissive = import_autokey(&directory.0, AutoKeyImportMode::Permissive);
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(matches!(strict, Err(AutoKeyImportError::Io { .. })));
        let imported = permissive.unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.report.skipped, 1);
        assert_eq!(imported.report.needs_review, 1);
        assert_eq!(imported.report.warnings[0].entry, "000-private");
        assert!(imported.report.warnings[0]
            .reason
            .contains("Permission denied"));
        assert!(imported.config.expansion[0].replacement.contains("Kept"));

        fs::remove_file(directory.0.join("available.txt")).unwrap();
        fs::remove_file(directory.0.join(".available.json")).unwrap();
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0)).unwrap();
        let only_unreadable = import_autokey(&directory.0, AutoKeyImportMode::Permissive);
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o700)).unwrap();
        let only_unreadable = only_unreadable.unwrap();
        assert_eq!(only_unreadable.report.imported, 0);
        assert_eq!(only_unreadable.report.skipped, 1);
        assert!(only_unreadable.config.expansion.is_empty());
    }

    #[test]
    fn imports_autokey_same_basename_json_sidecars() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("_LAneeded.txt"), "Useful text").unwrap();
        fs::write(directory.0.join("_LAneeded.json"), metadata(&[";needed"])).unwrap();

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Strict).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.config.expansion[0].trigger, ";needed");
        assert_eq!(imported.config.expansion[0].replacement, "Useful text");
    }

    #[test]
    fn permissive_import_skips_missing_and_malformed_sidecars_and_continues() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("missing.txt"), "no metadata").unwrap();
        fs::write(directory.0.join("broken.txt"), "bad metadata").unwrap();
        fs::write(directory.0.join(".broken.json"), "{").unwrap();
        directory.phrase("valid.txt", "valid phrase", &metadata(&[";valid"]));

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.report.skipped, 2);
        assert_eq!(imported.report.needs_review, 2);
        assert_eq!(
            imported.report.unsupported, 0,
            "missing and malformed source metadata are skipped entries, not unsupported phrase behavior"
        );
        assert_eq!(imported.report.warnings.len(), 2);
        assert!(imported.report.warnings[0].entry.contains("json"));
        assert!(!imported.report.warnings[0].remediation.is_empty());
        assert!(imported.report.warnings[1].entry.contains("json"));
        assert!(matches!(
            import_autokey(&directory.0, AutoKeyImportMode::Strict),
            Err(AutoKeyImportError::Metadata { .. })
        ));
    }

    #[test]
    fn permissive_import_skips_individually_invalid_expansions_and_continues() {
        let directory = TestDirectory::new();
        directory.phrase("invalid.txt", "invalid trigger", &metadata(&[""]));
        directory.phrase("valid.txt", "valid phrase", &metadata(&[";valid"]));

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.report.skipped, 1);
        assert_eq!(imported.report.needs_review, 1);
        assert_eq!(imported.config.expansion[0].trigger, ";valid");
        assert_eq!(imported.report.warnings[0].entry, "invalid.txt");
        assert!(imported.report.warnings[0].reason.contains("empty trigger"));
        assert!(!imported.report.warnings[0].remediation.is_empty());
        assert!(matches!(
            import_autokey(&directory.0, AutoKeyImportMode::Strict),
            Err(AutoKeyImportError::StrictRejected { entry, .. }) if entry == "invalid.txt"
        ));
    }

    #[test]
    fn preserves_non_wayexpand_template_syntax_as_literal_text() {
        let directory = TestDirectory::new();
        directory.phrase(
            "ticket.txt",
            "{{ticket.requester.name}} <cursor>",
            &metadata(&[";ticket"]),
        );

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Strict).unwrap();
        assert_eq!(
            imported.config.expansion[0].replacement,
            "{{{{ticket.requester.name}}}} {{cursor}}"
        );
    }

    #[test]
    fn permissive_import_reports_and_skips_duplicate_triggers() {
        let directory = TestDirectory::new();
        directory.phrase("first.txt", "First", &metadata(&[";same"]));
        directory.phrase("second.txt", "Second", &metadata(&[";same"]));

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.report.unsupported, 1);
        assert_eq!(imported.config.expansion[0].replacement, "First");
        assert!(imported.report.warnings[0]
            .reason
            .contains("duplicate AutoKey abbreviation"));
        assert!(matches!(
            import_autokey(&directory.0, AutoKeyImportMode::Strict),
            Err(AutoKeyImportError::DuplicateTrigger { .. })
        ));
    }

    #[test]
    fn directory_import_skips_appledouble_phrase_files() {
        let directory = TestDirectory::new();
        directory.phrase("greeting.txt", "Hello", &metadata(&[";hello"]));
        fs::write(directory.0.join("._greeting.txt"), "resource fork").unwrap();

        let imported = import_autokey(&directory.0, AutoKeyImportMode::Strict).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(imported.config.expansion[0].trigger, ";hello");
    }

    #[test]
    fn executable_dynamic_macros_are_skipped_and_strict_mode_rejects_them() {
        let directory = TestDirectory::new();
        let sentinel = directory.0.join("command-must-not-run");
        let path = directory.phrase(
            "command.txt",
            &format!("Result: <system command='touch {}'>", sentinel.display()),
            &metadata(&[";cmd"]),
        );
        let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
        assert!(imported.config.expansion.is_empty());
        assert_eq!(imported.report.unsupported, 1);
        assert!(imported.report.warnings[0]
            .reason
            .contains("dynamic AutoKey macro"));
        assert!(!sentinel.exists());
        assert!(matches!(
            import_autokey(path, AutoKeyImportMode::Strict),
            Err(AutoKeyImportError::StrictRejected { .. })
        ));
    }

    #[test]
    fn filtered_hotkey_and_case_insensitive_phrases_are_not_silently_broadened() {
        for (case, reason) in [
            ("filter", "window filters"),
            ("hotkey", "hotkey"),
            ("case-insensitive", "case-insensitive"),
        ] {
            let directory = TestDirectory::new();
            let mut metadata = serde_json::json!({
                "type": "phrase",
                "modes": [1],
                "abbreviation": {
                    "abbreviations": [";x"],
                    "immediate": false,
                    "ignoreCase": false,
                    "backspace": true,
                    "triggerInside": false
                }
            });
            match case {
                "filter" => metadata["filter"] = serde_json::json!({"regex": "Terminal"}),
                "hotkey" => {
                    metadata["hotkey"] = serde_json::json!({"hotKey": "<f12>", "modifiers": []})
                }
                "case-insensitive" => metadata["abbreviation"]["ignoreCase"] = true.into(),
                _ => unreachable!(),
            }
            directory.phrase("phrase.txt", "text", &metadata.to_string());
            let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
            assert_eq!(imported.report.unsupported, 1);
            assert!(imported.report.warnings[0].reason.contains(reason));
        }
    }

    #[test]
    fn unsupported_modes_and_custom_word_boundaries_are_reported() {
        for (field, value, reason) in [
            ("modes", serde_json::json!([3]), "trigger modes"),
            (
                "wordChars",
                serde_json::json!("[a-z]"),
                "boundary characters",
            ),
        ] {
            let directory = TestDirectory::new();
            let mut data: serde_json::Value = serde_json::from_str(&metadata(&[";x"])).unwrap();
            if field == "modes" {
                data[field] = value;
            } else {
                data["abbreviation"][field] = value;
            }
            directory.phrase("phrase.txt", "text", &data.to_string());
            let imported = import_autokey(&directory.0, AutoKeyImportMode::Permissive).unwrap();
            assert_eq!(imported.report.unsupported, 1);
            assert!(imported.report.warnings[0].reason.contains(reason));
        }
    }

    #[test]
    fn dynamic_macro_detection_does_not_match_longer_literal_tags() {
        let directory = TestDirectory::new();
        directory.phrase(
            "phrase.txt",
            "Use <systematic> planning",
            &metadata(&[";x"]),
        );
        let imported = import_autokey(&directory.0, AutoKeyImportMode::Strict).unwrap();
        assert_eq!(imported.report.imported, 1);
        assert_eq!(
            imported.config.expansion[0].replacement,
            "Use <systematic> planning"
        );
    }

    #[test]
    fn symlink_phrase_inputs_are_never_followed() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let target = directory.0.join("target.txt");
        fs::write(&target, "secret phrase").unwrap();
        symlink(&target, directory.0.join("link.txt")).unwrap();
        assert!(matches!(
            import_autokey(directory.0.join("link.txt"), AutoKeyImportMode::Permissive),
            Err(AutoKeyImportError::UnsafePath(_))
        ));
    }
}
