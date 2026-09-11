use crate::{Config, ConfigError, ExpansionConfig, MatchMode, Settings};
use serde::Deserialize;
use std::{fs, path::Path};
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
    #[error("could not parse Espanso YAML: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("imported configuration is invalid: {0}")]
    Invalid(#[from] ConfigError),
}

pub fn import_espanso(path: impl AsRef<Path>) -> Result<EspansoImport, MigrationError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| MigrationError::Read {
        path: path.display().to_string(),
        source,
    })?;
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
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
        });
    }
    let config = Config {
        expansion,
        hotkey: Vec::new(),
        settings: Settings::default(),
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
}
