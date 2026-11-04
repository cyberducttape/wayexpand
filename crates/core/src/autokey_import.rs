//! Safe import of static AutoKey phrase files.
//!
//! AutoKey stores a phrase in a UTF-8 `.txt` file and its triggers/settings in
//! a JSON sidecar named `.<phrase>.json`. Python scripts and executable
//! macros are intentionally never run during import.

use crate::{
    config::{OrganizationPolicy, MAX_CONFIG_BYTES},
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
    if metadata.is_dir() {
        collect_phrase_files(path, 0, &mut phrases)?;
    } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "txt") {
        phrases.push(path.to_path_buf());
    } else {
        return Err(AutoKeyImportError::UnsafePath(path.display().to_string()));
    }
    phrases.sort();
    if phrases.is_empty() {
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
    let mut report = AutoKeyImportReport::default();
    let mut triggers = HashSet::new();
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
                let duplicate = std::iter::once(&expansion.trigger)
                    .chain(&expansion.aliases)
                    .find(|trigger| triggers.contains(*trigger));
                if let Some(trigger) = duplicate {
                    if mode == AutoKeyImportMode::Strict {
                        return Err(AutoKeyImportError::DuplicateTrigger {
                            path: relative.clone(),
                            trigger: trigger.clone(),
                        });
                    }
                    report.unsupported += 1;
                    report.skipped += 1;
                    report.needs_review += 1;
                    report.warnings.push(AutoKeyImportWarning {
                        entry: relative,
                        reason: format!(
                            "duplicate AutoKey abbreviation {trigger:?}; skipped to avoid ambiguous matching"
                        ),
                        remediation: "Rename the duplicate abbreviation or merge the two phrases manually."
                            .into(),
                    });
                    continue;
                }
                triggers.extend(
                    std::iter::once(expansion.trigger.clone())
                        .chain(expansion.aliases.iter().cloned()),
                );
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
    directory: &Path,
    depth: usize,
    phrases: &mut Vec<PathBuf>,
) -> Result<(), AutoKeyImportError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(AutoKeyImportError::TooManyPhrases);
    }
    for entry in fs::read_dir(directory).map_err(|source| io_error(directory, source))? {
        let entry = entry.map_err(|source| io_error(directory, source))?;
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
        let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_phrase_files(&path, depth + 1, phrases)?;
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
