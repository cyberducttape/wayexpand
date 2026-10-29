use crate::{
    config::{OrganizationPolicy, MAX_CONFIG_BYTES},
    Config, ConfigError, ExpansionConfig, MatchMode, Settings,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use thiserror::Error;

mod espanso_conversion;
use espanso_conversion::{
    convert_espanso_cursor_marker, convert_espanso_form, convert_espanso_variables,
};

#[derive(Debug, Deserialize)]
struct EspansoDocument {
    #[serde(default)]
    matches: Vec<EspansoMatch>,
    #[serde(default)]
    global_vars: EspansoGlobalVars,
    /// Unmapped options. Only their names are reported, so the values are
    /// skipped unparsed: any YAML value (integer keys, `.nan`) is accepted.
    #[serde(flatten)]
    extra: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum EspansoGlobalVars {
    #[default]
    Empty,
    List(Vec<EspansoVariable>),
    Other(serde::de::IgnoredAny),
}

impl EspansoGlobalVars {
    fn variables(&self) -> &[EspansoVariable] {
        match self {
            Self::List(variables) => variables,
            Self::Empty | Self::Other(_) => &[],
        }
    }

    fn is_unmapped(&self) -> bool {
        matches!(self, Self::Other(_))
    }
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
    /// Espanso's shorthand form layout (`[[field_name]]`).
    form: Option<String>,
    #[serde(default)]
    form_fields: BTreeMap<String, EspansoFormField>,
    #[serde(default)]
    vars: Vec<EspansoVariable>,
    label: Option<String>,
    #[serde(default)]
    propagate_case: bool,
    /// Unmapped options. Only their names are reported, so the values are
    /// skipped unparsed: any YAML value (integer keys, `.nan`) is accepted.
    #[serde(flatten)]
    extra: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Debug, Deserialize)]
struct EspansoVariable {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    params: EspansoVariableParams,
    #[serde(flatten)]
    extra: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Debug, Default, Deserialize)]
struct EspansoVariableParams {
    echo: Option<String>,
    format: Option<String>,
    offset: Option<i64>,
    values: Option<EspansoChoiceValues>,
    choices: Option<EspansoChoiceValues>,
    trigger: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EspansoChoiceValues {
    List(Vec<EspansoOption>),
    Other(serde::de::IgnoredAny),
}

#[derive(Debug)]
enum EspansoOption {
    Text(String),
    Unsupported,
}

impl<'de> Deserialize<'de> for EspansoOption {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptionVisitor;

        impl<'de> serde::de::Visitor<'de> for OptionVisitor {
            type Value = EspansoOption;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string choice or an unsupported structured choice")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(EspansoOption::Text(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(EspansoOption::Text(value))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(EspansoOption::Unsupported)
            }

            fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
            where
                S: serde::de::SeqAccess<'de>,
            {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(EspansoOption::Unsupported)
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(EspansoOption::Unsupported)
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(EspansoOption::Unsupported)
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(EspansoOption::Unsupported)
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(EspansoOption::Unsupported)
            }
        }

        deserializer.deserialize_any(OptionVisitor)
    }
}

#[derive(Debug, Default, Deserialize)]
struct EspansoFormField {
    #[serde(rename = "type")]
    kind: Option<String>,
    values: Option<EspansoFormValues>,
    default: Option<String>,
    multiline: Option<bool>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EspansoFormValues {
    List(Vec<String>),
    Lines(String),
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
    #[error("Espanso files contain duplicate trigger {trigger:?} in {path}")]
    DuplicateTrigger { path: String, trigger: String },
    #[error("could not parse Espanso YAML: {0}")]
    Parse(#[from] serde_saphyr::Error),
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
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => import_espanso_directory(path, mode),
        Ok(_) => import_espanso_file_with_mode(path, mode),
        Err(source) => Err(MigrationError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn import_espanso_file_with_mode(
    path: &Path,
    mode: EspansoImportMode,
) -> Result<EspansoImport, MigrationError> {
    import_espanso_file_with_limit(path, mode, MAX_CONFIG_BYTES, true).map(|(imported, _)| imported)
}

fn import_espanso_file_with_limit(
    path: &Path,
    mode: EspansoImportMode,
    maximum_bytes: usize,
    validate: bool,
) -> Result<(EspansoImport, usize), MigrationError> {
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
    if metadata.len() > maximum_bytes as u64 {
        return Err(MigrationError::TooLarge {
            path: path.display().to_string(),
            length: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            maximum: maximum_bytes,
        });
    }
    // Cap the read itself too: metadata can be stale or, for a non-regular
    // file, misleading about how many bytes are actually available.
    let mut bytes = Vec::new();
    file.by_ref()
        .take(maximum_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > maximum_bytes {
        return Err(MigrationError::TooLarge {
            path: path.display().to_string(),
            length: bytes.len(),
            maximum: maximum_bytes,
        });
    }
    let bytes_read = bytes.len();
    let text = String::from_utf8(bytes).map_err(|error| {
        read_error(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.utf8_error(),
        ))
    })?;
    // The byte limit bounds the source document size; the YAML deserializer
    // also applies a recursion limit and a node budget (which stops alias
    // "billion laughs" expansion). Non-finite floats (`.nan`, `.inf`) are
    // valid YAML and may appear in options this importer skips, so they are
    // passed through instead of failing the whole import.
    let mut options = serde_saphyr::Options::default();
    options.non_finite_float_policy = serde_saphyr::NonFiniteFloatPolicy::PassThrough;
    let document: EspansoDocument = serde_saphyr::from_str_with_options(&text, options)?;
    let mut expansion = Vec::with_capacity(document.matches.len());
    let mut report = EspansoImportReport::default();
    if document.global_vars.is_unmapped() {
        if mode == EspansoImportMode::Strict {
            return Err(MigrationError::StrictRejected {
                trigger: "(file)".into(),
                details: "top-level `global_vars` is not in the supported list format".into(),
            });
        }
        report.warnings.push(EspansoImportWarning {
            trigger: "(file)".into(),
            details: vec!["top-level `global_vars` is not in the supported list format".into()],
        });
    }
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
        let had_replace = item.replace.is_some();
        let (replacement, form_details) = match (item.replace, item.form) {
            (Some(_), Some(_)) => {
                report.unsupported += 1;
                report.unsupported_matches.push(EspansoUnsupportedMatch {
                    trigger,
                    reason:
                        "match defines both `replace` and `form`; automatic conversion is ambiguous"
                            .into(),
                });
                continue;
            }
            (Some(replacement), None) if item.form_fields.is_empty() => (replacement, Vec::new()),
            (Some(_), None) => {
                report.unsupported += 1;
                report.unsupported_matches.push(EspansoUnsupportedMatch {
                    trigger,
                    reason: "`form_fields` is present without an Espanso `form` layout".into(),
                });
                continue;
            }
            (None, Some(form)) if item.vars.is_empty() => {
                match convert_espanso_form(&form, &item.form_fields) {
                    Ok((replacement, details)) => {
                        if mode == EspansoImportMode::Strict && !details.is_empty() {
                            report.unsupported += 1;
                            report.unsupported_matches.push(EspansoUnsupportedMatch {
                                trigger,
                                reason: format!(
                                    "strict mode discarded the form because {}",
                                    details.join(", ")
                                ),
                            });
                            continue;
                        }
                        (replacement, details)
                    }
                    Err(reason) => {
                        report.unsupported += 1;
                        report
                            .unsupported_matches
                            .push(EspansoUnsupportedMatch { trigger, reason });
                        continue;
                    }
                }
            }
            (None, Some(_)) => {
                report.unsupported += 1;
                report.unsupported_matches.push(EspansoUnsupportedMatch {
                    trigger,
                    reason: "forms combined with dynamic variables need manual conversion".into(),
                });
                continue;
            }
            (None, None) => {
                report.unsupported += 1;
                report.unsupported_matches.push(EspansoUnsupportedMatch {
                    trigger,
                    reason: if item.vars.is_empty() {
                        "match has no static `replace` value".into()
                    } else {
                        "match has dynamic variables but no `replace` or `form` layout".into()
                    },
                });
                continue;
            }
        };
        let replacement = match convert_espanso_cursor_marker(&replacement) {
            Ok(replacement) => replacement,
            Err(reason) => {
                report.unsupported += 1;
                report
                    .unsupported_matches
                    .push(EspansoUnsupportedMatch { trigger, reason });
                continue;
            }
        };
        let (replacement, dynamic_details) = if had_replace {
            match convert_espanso_variables(
                &replacement,
                document.global_vars.variables().iter().chain(&item.vars),
            ) {
                Ok(converted) => converted,
                Err(reason) => {
                    report.unsupported += 1;
                    report
                        .unsupported_matches
                        .push(EspansoUnsupportedMatch { trigger, reason });
                    continue;
                }
            }
        } else {
            (replacement, form_details)
        };
        let mut details = item
            .extra
            .keys()
            .map(|key| format!("option `{key}` is not mapped; review its semantics"))
            .collect::<Vec<_>>();
        details.extend(dynamic_details);
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
            output_file: None,
        });
    }
    let mut config = Config {
        expansion,
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    if validate {
        remove_unresolved_nested_matches(&mut config, &mut report);
        config.validate()?;
    }
    let skipped = report.unsupported;
    Ok((
        EspansoImport {
            config,
            report,
            skipped,
        },
        bytes_read,
    ))
}

fn import_espanso_directory(
    root: &Path,
    mode: EspansoImportMode,
) -> Result<EspansoImport, MigrationError> {
    const MAX_MATCH_FILES: usize = 1_024;
    let root_metadata = fs::symlink_metadata(root).map_err(|source| MigrationError::Read {
        path: root.display().to_string(),
        source,
    })?;
    if !root_metadata.file_type().is_dir() {
        return Err(MigrationError::NotRegular {
            path: root.display().to_string(),
        });
    }
    let match_dir = root.join("match");
    let source_dir = match fs::symlink_metadata(&match_dir) {
        Ok(metadata) if metadata.file_type().is_dir() => match_dir,
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(MigrationError::NotRegular {
                path: match_dir.display().to_string(),
            });
        }
        Ok(_) => root.to_path_buf(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => root.to_path_buf(),
        Err(source) => {
            return Err(MigrationError::Read {
                path: match_dir.display().to_string(),
                source,
            });
        }
    };
    let mut files = Vec::new();
    collect_espanso_match_files(&source_dir, &mut files, MAX_MATCH_FILES)?;
    files.sort();
    if files.is_empty() {
        return Err(MigrationError::Read {
            path: source_dir.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no regular .yml or .yaml match files found",
            ),
        });
    }
    let mut total_bytes = 0u64;
    for file in &files {
        let metadata = fs::symlink_metadata(file).map_err(|source| MigrationError::Read {
            path: file.display().to_string(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(MigrationError::NotRegular {
                path: file.display().to_string(),
            });
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_CONFIG_BYTES as u64 {
            return Err(MigrationError::TooLarge {
                path: source_dir.display().to_string(),
                length: usize::try_from(total_bytes).unwrap_or(usize::MAX),
                maximum: MAX_CONFIG_BYTES,
            });
        }
    }

    let mut combined = EspansoImport {
        config: Config {
            expansion: Vec::new(),
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        },
        report: EspansoImportReport::default(),
        skipped: 0,
    };
    let mut triggers = std::collections::HashSet::new();
    let mut bytes_consumed = 0usize;
    for file in files {
        let remaining = MAX_CONFIG_BYTES.saturating_sub(bytes_consumed);
        let (imported, bytes_read) = import_espanso_file_with_limit(&file, mode, remaining, false)?;
        bytes_consumed = bytes_consumed.saturating_add(bytes_read);
        combined.report.fully_migrated += imported.report.fully_migrated;
        combined.report.migrated_with_warnings += imported.report.migrated_with_warnings;
        combined.report.unsupported += imported.report.unsupported;
        combined.report.warnings.extend(imported.report.warnings);
        combined
            .report
            .unsupported_matches
            .extend(imported.report.unsupported_matches);
        for expansion in imported.config.expansion {
            let mut all_triggers = std::iter::once(&expansion.trigger).chain(&expansion.aliases);
            if let Some(conflict) = all_triggers.find(|trigger| triggers.contains(*trigger)) {
                return Err(MigrationError::DuplicateTrigger {
                    path: file.display().to_string(),
                    trigger: conflict.clone(),
                });
            }
            triggers.extend(
                std::iter::once(expansion.trigger.clone()).chain(expansion.aliases.iter().cloned()),
            );
            combined.config.expansion.push(expansion);
        }
    }
    remove_unresolved_nested_matches(&mut combined.config, &mut combined.report);
    combined.config.validate()?;
    combined.skipped = combined.report.unsupported;
    Ok(combined)
}

fn remove_unresolved_nested_matches(config: &mut Config, report: &mut EspansoImportReport) {
    let mut includable = std::collections::HashSet::new();
    for expansion in &config.expansion {
        if crate::form_fields(&expansion.replacement).is_ok_and(|fields| fields.is_empty()) {
            includable.insert(expansion.trigger.clone());
            includable.extend(expansion.aliases.iter().cloned());
        }
    }
    let warned: std::collections::HashSet<_> = report
        .warnings
        .iter()
        .map(|warning| warning.trigger.clone())
        .collect();
    let mut retained = Vec::with_capacity(config.expansion.len());
    for expansion in config.expansion.drain(..) {
        let missing = crate::template_variables(&expansion.replacement)
            .into_iter()
            .filter_map(|name| name.strip_prefix("snippet:"))
            .map(str::trim)
            .find(|trigger| !includable.contains(*trigger));
        if let Some(trigger) = missing {
            if warned.contains(&expansion.trigger) {
                report.migrated_with_warnings = report.migrated_with_warnings.saturating_sub(1);
            } else {
                report.fully_migrated = report.fully_migrated.saturating_sub(1);
            }
            report.unsupported += 1;
            report.unsupported_matches.push(EspansoUnsupportedMatch {
                trigger: expansion.trigger,
                reason: format!(
                    "nested match references {trigger:?}, which was not imported as a static snippet"
                ),
            });
        } else {
            retained.push(expansion);
        }
    }
    config.expansion = retained;
}

fn collect_espanso_match_files(
    directory: &Path,
    files: &mut Vec<std::path::PathBuf>,
    maximum_files: usize,
) -> Result<(), MigrationError> {
    fn collect(
        directory: &Path,
        files: &mut Vec<std::path::PathBuf>,
        maximum_files: usize,
        depth: usize,
        total_entries: &mut usize,
    ) -> Result<(), MigrationError> {
        const MAX_DIRECTORY_DEPTH: usize = 32;
        const MAX_ENTRIES_PER_DIRECTORY: usize = 4_096;
        const MAX_TOTAL_ENTRIES: usize = 16_384;
        if depth > MAX_DIRECTORY_DEPTH {
            return Err(MigrationError::Read {
                path: directory.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Espanso match directories exceed the nesting limit",
                ),
            });
        }
        let entries = fs::read_dir(directory).map_err(|source| MigrationError::Read {
            path: directory.display().to_string(),
            source,
        })?;
        let mut entries = entries
            .map(|entry| {
                entry.map_err(|source| MigrationError::Read {
                    path: directory.display().to_string(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if entries.len() > MAX_ENTRIES_PER_DIRECTORY {
            return Err(MigrationError::Read {
                path: directory.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Espanso match directory contains too many entries",
                ),
            });
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            *total_entries += 1;
            if *total_entries > MAX_TOTAL_ENTRIES {
                return Err(MigrationError::Read {
                    path: directory.display().to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Espanso match tree contains too many entries",
                    ),
                });
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|source| MigrationError::Read {
                path: path.display().to_string(),
                source,
            })?;
            if metadata.file_type().is_symlink() {
                let is_match_file = path
                    .extension()
                    .is_some_and(|extension| extension == "yml" || extension == "yaml");
                let points_to_directory = fs::metadata(&path).is_ok_and(|target| target.is_dir());
                if is_match_file || points_to_directory {
                    return Err(MigrationError::NotRegular {
                        path: path.display().to_string(),
                    });
                }
                continue;
            }
            if metadata.file_type().is_dir() {
                collect(&path, files, maximum_files, depth + 1, total_entries)?;
            } else if metadata.file_type().is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "yml" || extension == "yaml")
            {
                files.push(path);
                if files.len() > maximum_files {
                    return Err(MigrationError::TooLarge {
                        path: directory.display().to_string(),
                        length: files.len(),
                        maximum: maximum_files,
                    });
                }
            }
        }
        Ok(())
    }
    // Keep recursion bounded so an adversarial directory tree cannot exhaust
    // the importer's stack.
    let mut total_entries = 0;
    collect(directory, files, maximum_files, 0, &mut total_entries)
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

    fn import_text(name: &str, text: &str) -> Result<EspansoImport, MigrationError> {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-espanso-{name}-{}.yml",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        let imported = import_espanso(&path);
        fs::remove_file(path).unwrap();
        imported
    }

    #[test]
    fn any_value_in_an_unmapped_option_is_accepted() {
        // Only option names are reported, so values the old YAML parser
        // accepted (integer keys, non-finite floats) must not fail the import.
        let imported = import_text(
            "unmapped-values",
            "threshold: .nan
limit: -.inf
matches:
  - trigger: ':pick'
    replace: chosen
    vars:
      - name: x
        type: choice
        params: {values: {1: one, 2: two}}
",
        )
        .unwrap();
        assert_eq!(imported.config.expansion.len(), 1);
        assert_eq!(imported.report.migrated_with_warnings, 0);
        assert_eq!(imported.report.warnings.len(), 2);
    }

    #[test]
    fn alias_expansion_bombs_are_rejected() {
        let mut text = String::from("a: &a [lol, lol, lol, lol, lol, lol, lol, lol, lol]\n");
        let mut previous = 'a';
        for name in "bcdefghi".chars() {
            let items = vec![format!("*{previous}"); 9].join(", ");
            text.push_str(&format!("{name}: &{name} [{items}]\n"));
            previous = name;
        }
        text.push_str("matches: []\n");
        assert!(matches!(
            import_text("alias-bomb", &text),
            Err(MigrationError::Parse(_))
        ));
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

    #[test]
    fn imports_safe_dynamic_espanso_variables_as_wayexpand_templates() {
        let imported = import_text(
            "dynamic-safe",
            r#"matches:
  - trigger: ':incident-date'
    replace: "{{greeting}} — {{when}} — {{severity}} — {{quote}}"
    vars:
      - name: greeting
        type: echo
        params:
          echo: "Hello {{username}}"
      - name: when
        type: date
        params:
          format: "%Y-%m-%d"
          offset: 86400
      - name: severity
        type: choice
        params:
          values: [Low, High]
      - name: quote
        type: random
        params:
          choices: [First, Second]
"#,
        )
        .unwrap();

        assert_eq!(imported.config.expansion.len(), 1);
        assert_eq!(imported.report.migrated_with_warnings, 1);
        let replacement = &imported.config.expansion[0].replacement;
        assert!(replacement.contains("Hello {{{{username}}}}"));
        assert!(replacement.contains("{{local_date+1d:%Y-%m-%d}}"));
        assert!(replacement.contains("{{choice:severity=Low|High}}"));
        assert!(replacement.contains("{{random:First|Second}}"));
    }

    #[test]
    fn imports_supported_global_echo_variables() {
        let imported = import_text(
            "global-echo",
            r#"global_vars:
  - name: greeting
    type: echo
    params:
      echo: "Hello"
matches:
  - trigger: ':hello'
    replace: "{{greeting}}, world!"
"#,
        )
        .unwrap();
        assert_eq!(imported.config.expansion[0].replacement, "Hello, world!");
        assert_eq!(imported.report.fully_migrated, 1);
    }

    #[test]
    fn imports_nested_match_variables_and_cursor_markers() {
        let imported = import_text(
            "nested-match",
            r#"matches:
  - trigger: ':one'
    replace: inner
  - trigger: ':nested'
    replace: "Before {{output}}$|$After"
    vars:
      - name: output
        type: match
        params:
          trigger: ':one'
"#,
        )
        .unwrap();
        let replacement = &imported.config.expansion[1].replacement;
        assert!(replacement.contains("{{snippet::one}}"));
        assert!(replacement.contains("{{cursor}}"));
        assert_eq!(imported.report.fully_migrated, 2);
    }

    #[test]
    fn multiple_espanso_cursor_markers_are_reported_as_unsupported() {
        let imported = import_text(
            "multi-cursor",
            "matches:\n  - trigger: ':bad'\n    replace: '$|$middle$|$'\n",
        )
        .unwrap();
        assert!(imported.config.expansion.is_empty());
        assert_eq!(imported.report.unsupported, 1);
        assert!(imported.report.unsupported_matches[0]
            .reason
            .contains("more than one"));
    }

    #[test]
    fn nested_match_with_an_unimported_target_is_reported_without_aborting() {
        let imported = import_text(
            "nested-unsupported-target",
            r#"matches:
  - trigger: ':code'
    replace: "{{output}}"
    vars:
      - name: output
        type: shell
        params:
          cmd: "unsafe command"
  - trigger: ':nested'
    replace: "{{output}}"
    vars:
      - name: output
        type: match
        params:
          trigger: ':code'
"#,
        )
        .unwrap();
        assert!(imported.config.expansion.is_empty());
        assert_eq!(imported.report.unsupported, 2);
        assert!(imported.report.unsupported_matches[1]
            .reason
            .contains("not imported as a static snippet"));
    }

    #[test]
    fn imports_espanso_shorthand_forms_and_choice_fields() {
        let imported = import_text(
            "form",
            r#"matches:
  - trigger: ':incident'
    form: |
      Incident for [[hostname]]
      Severity: [[severity]]
    form_fields:
      hostname:
        default: node-1
      severity:
        type: choice
        values: |
          Low
          High
        default: High
"#,
        )
        .unwrap();

        let replacement = &imported.config.expansion[0].replacement;
        assert!(replacement.contains("{{field:hostname=node-1}}"));
        assert!(replacement.contains("{{choice:severity=Low|High;default=High}}"));
        assert_eq!(crate::form_fields(replacement).unwrap().len(), 2);
        assert_eq!(imported.report.fully_migrated, 1);
    }

    #[test]
    fn clipboard_and_executable_dynamic_variables_stay_unimported() {
        let imported = import_text(
            "unsafe-dynamic",
            r#"matches:
  - trigger: ':clipboard'
    replace: "{{clip}}"
    vars:
      - name: clip
        type: clipboard
  - trigger: ':shell'
    replace: "{{output}}"
    vars:
      - name: output
        type: shell
        params:
          cmd: "touch /tmp/should-not-run"
"#,
        )
        .unwrap();

        assert!(imported.config.expansion.is_empty());
        assert_eq!(imported.report.unsupported, 2);
        assert!(imported.report.unsupported_matches[0]
            .reason
            .contains("explicit clipboard enablement"));
        assert!(imported.report.unsupported_matches[1]
            .reason
            .contains("executes external code"));
    }

    #[test]
    fn imports_nested_espanso_match_directories_in_stable_order() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-espanso-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let match_dir = root.join("match");
        let package_dir = match_dir.join("packages").join("team");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            match_dir.join("base.yml"),
            "matches:\n  - trigger: ':base'\n    replace: Base\n",
        )
        .unwrap();
        fs::write(
            package_dir.join("support.yaml"),
            "matches:\n  - trigger: ':support'\n    form: 'Hello [[name]]'\n  - trigger: ':nested'\n    replace: '{{output}}'\n    vars:\n      - name: output\n        type: match\n        params:\n          trigger: ':base'\n",
        )
        .unwrap();
        fs::write(package_dir.join("README.md"), "ignored").unwrap();

        let imported = import_espanso(&root).unwrap();
        assert_eq!(
            imported
                .config
                .expansion
                .iter()
                .map(|expansion| expansion.trigger.as_str())
                .collect::<Vec<_>>(),
            [":base", ":support", ":nested"]
        );
        assert!(imported.config.expansion[1]
            .replacement
            .contains("{{field:name}}"));
        assert!(imported.config.expansion[2]
            .replacement
            .contains("{{snippet::base}}"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_import_refuses_yaml_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "wayexpand-espanso-tree-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let outside = root.with_extension("yml");
        fs::write(&outside, "matches: []\n").unwrap();
        symlink(&outside, root.join("linked.yml")).unwrap();
        assert!(matches!(
            import_espanso(&root),
            Err(MigrationError::NotRegular { .. })
        ));
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }
}
