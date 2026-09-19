use crate::{config::{MAX_CONFIG_BYTES, OrganizationPolicy}, Config, ConfigError, ExpansionConfig, MatchMode, Settings};
use serde::Deserialize;
use std::{fs, io::Read, path::Path};
use thiserror::Error;

#[derive(Debug, Deserialize)]
struct EspansoDocument {
    #[serde(default)]
    matches: Vec<EspansoMatch>,
}

#[derive(Debug, Deserialize)]
struct EspansoMatch {
    trigger: String,
    replace: Option<String>,
    label: Option<String>,
}

#[derive(Debug)]
pub struct EspansoImport {
    pub config: Config,
    pub skipped: usize,
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
    #[error("could not parse Espanso YAML: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("imported configuration is invalid: {0}")]
    Invalid(#[from] ConfigError),
}

pub fn import_espanso(path: impl AsRef<Path>) -> Result<EspansoImport, MigrationError> {
    let path = path.as_ref();
    let read_error = |source: std::io::Error| MigrationError::Read {
        path: path.display().to_string(),
        source,
    };
    let mut file = fs::File::open(path).map_err(read_error)?;
    let metadata = file.metadata().map_err(read_error)?;
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
    // serde_yaml resolves anchors/aliases; the size cap above bounds how much
    // expansion a crafted "billion laughs"-style document can achieve here.
    let document: EspansoDocument = serde_yaml::from_str(&text)?;
    let mut expansion = Vec::with_capacity(document.matches.len());
    let mut skipped = 0;
    for item in document.matches {
        let Some(replacement) = item.replace else {
            skipped += 1;
            continue;
        };
        expansion.push(ExpansionConfig {
            trigger: item.trigger,
            replacement,
            description: item.label.unwrap_or_default(),
            tags: vec!["imported".into()],
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            // Espanso's own `propagate_case` match option is not mapped
            // here; imported snippets keep their replacement text as-is.
            propagate_case: false,
        });
    }
    let config = Config {
        expansion,
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.validate()?;
    Ok(EspansoImport { config, skipped })
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
}
