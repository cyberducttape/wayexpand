use crate::Config;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

const MANIFEST_FILE: &str = "wayexpand-pack.toml";
const SNIPPETS_DIR: &str = "snippets";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub name: String,
    pub publisher: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackInspection {
    pub manifest: PackManifest,
    pub snippet_files: usize,
    pub expansion_count: usize,
    pub hotkey_count: usize,
    pub command_count: usize,
}

#[derive(Debug, Error)]
pub enum PackError {
    #[error("pack path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("pack manifest is invalid: {0}")]
    Manifest(#[from] toml::de::Error),
    #[error("pack format version {0} is unsupported")]
    UnsupportedFormat(u32),
    #[error("pack manifest field `{0}` is empty")]
    EmptyField(&'static str),
    #[error("pack has no snippets directory")]
    MissingSnippets,
    #[error("could not read pack file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("snippet file {path} is invalid: {source}")]
    Snippet {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("pack configuration is invalid: {0}")]
    InvalidConfig(String),
}

fn read_manifest(path: &Path) -> Result<PackManifest, PackError> {
    let manifest_path = path.join(MANIFEST_FILE);
    let source = fs::read_to_string(&manifest_path).map_err(|source| PackError::Read {
        path: manifest_path,
        source,
    })?;
    let manifest: PackManifest = toml::from_str(&source)?;
    if manifest.format_version != 1 {
        return Err(PackError::UnsupportedFormat(manifest.format_version));
    }
    for (value, field) in [
        (manifest.id.as_str(), "id"),
        (manifest.version.as_str(), "version"),
        (manifest.name.as_str(), "name"),
        (manifest.publisher.as_str(), "publisher"),
    ] {
        if value.trim().is_empty() {
            return Err(PackError::EmptyField(field));
        }
    }
    Ok(manifest)
}

fn read_snippet_configs(path: &Path) -> Result<Vec<(PathBuf, Config)>, PackError> {
    let snippets_path = path.join(SNIPPETS_DIR);
    if !snippets_path.is_dir() {
        return Err(PackError::MissingSnippets);
    }
    let mut files = fs::read_dir(&snippets_path)
        .map_err(|source| PackError::Read {
            path: snippets_path.clone(),
            source,
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| PackError::Read {
            path: snippets_path,
            source,
        })?;
    files.sort();
    let mut configs = Vec::new();
    for file in files {
        if file.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let source = fs::read_to_string(&file).map_err(|source| PackError::Read {
            path: file.clone(),
            source,
        })?;
        let config = toml::from_str(&source).map_err(|source| PackError::Snippet {
            path: file.clone(),
            source,
        })?;
        configs.push((file, config));
    }
    Ok(configs)
}

pub fn inspect_pack(path: impl AsRef<Path>) -> Result<PackInspection, PackError> {
    let path = path.as_ref();
    if !path.is_dir() {
        return Err(PackError::NotDirectory(path.to_owned()));
    }
    let manifest = read_manifest(path)?;
    let configs = read_snippet_configs(path)?;
    let expansion_count = configs
        .iter()
        .map(|(_, config)| config.expansion.len())
        .sum();
    let hotkey_count = configs.iter().map(|(_, config)| config.hotkey.len()).sum();
    let command_count = configs
        .iter()
        .map(|(_, config)| {
            config
                .expansion
                .iter()
                .filter(|item| item.command.is_some())
                .count()
                + config.hotkey.len()
        })
        .sum();
    Ok(PackInspection {
        manifest,
        snippet_files: configs.len(),
        expansion_count,
        hotkey_count,
        command_count,
    })
}

/// Load a pack for review. Commands are intentionally stripped from the
/// returned configuration; importing a pack must never grant execution
/// privileges implicitly.
pub fn import_pack(path: impl AsRef<Path>) -> Result<(PackInspection, Config, usize), PackError> {
    let path = path.as_ref();
    let inspection = inspect_pack(path)?;
    let configs = read_snippet_configs(path)?;
    let mut merged = Config {
        expansion: Vec::new(),
        hotkey: Vec::new(),
        settings: Default::default(),
        organization: Default::default(),
    };
    let mut disabled_commands = 0;
    for (_, mut config) in configs {
        for expansion in &mut config.expansion {
            if expansion.command.take().is_some() {
                disabled_commands += 1;
            }
        }
        disabled_commands += config.hotkey.len();
        config.hotkey.clear();
        merged.expansion.extend(config.expansion);
    }
    merged
        .validate()
        .map_err(|error| PackError::InvalidConfig(error.safe_summary()))?;
    Ok((inspection, merged, disabled_commands))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pack_path() -> PathBuf {
        std::env::temp_dir().join(format!("wayexpand-pack-{}", std::process::id()))
    }

    #[test]
    fn inspect_and_import_disable_commands() {
        let path = test_pack_path();
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        fs::write(
            path.join(MANIFEST_FILE),
            "format_version = 1\nid = \"demo\"\nversion = \"1.0.0\"\nname = \"Demo\"\npublisher = \"Test\"\n",
        )
        .unwrap();
        fs::write(
            path.join(SNIPPETS_DIR).join("main.toml"),
            "[[expansion]]\ntrigger = \":hello\"\nreplacement = \"Hello\"\n\n[[expansion]]\ntrigger = \":date\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"date\"\n",
        )
        .unwrap();

        let inspection = inspect_pack(&path).unwrap();
        assert_eq!(inspection.expansion_count, 2);
        assert_eq!(inspection.command_count, 1);
        let (_, config, disabled) = import_pack(&path).unwrap();
        assert_eq!(disabled, 1);
        assert!(config.expansion.iter().all(|item| item.command.is_none()));
        fs::remove_dir_all(path).unwrap();
    }
}
