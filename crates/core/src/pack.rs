use crate::Config;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use thiserror::Error;

const MANIFEST_FILE: &str = "wayexpand-pack.toml";
const SNIPPETS_DIR: &str = "snippets";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_SNIPPET_FILE_BYTES: usize = 1024 * 1024;
const MAX_SNIPPET_FILES: usize = 1024;
const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

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
    #[error("pack file {path} is not a regular file")]
    InvalidFile { path: PathBuf },
    #[error("pack file {path} exceeds the {maximum} byte limit")]
    FileTooLarge { path: PathBuf, maximum: usize },
    #[error("pack contains more than {maximum} snippet files")]
    TooManyFiles { maximum: usize },
    #[error("pack exceeds the {maximum} byte aggregate limit")]
    PackTooLarge { maximum: usize },
    #[error("pack file {path} is not valid UTF-8: {source}")]
    InvalidUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    #[error("snippet file {path} is invalid: {source}")]
    Snippet {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("pack configuration is invalid: {0}")]
    InvalidConfig(String),
}

fn read_bounded_text(path: &Path, maximum: usize) -> Result<String, PackError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| PackError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(PackError::InvalidFile {
            path: path.to_owned(),
        });
    }
    if metadata.len() > maximum as u64 {
        return Err(PackError::FileTooLarge {
            path: path.to_owned(),
            maximum,
        });
    }
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path).map_err(|source| PackError::Read {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((maximum as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| PackError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > maximum {
        return Err(PackError::FileTooLarge {
            path: path.to_owned(),
            maximum,
        });
    }
    String::from_utf8(bytes).map_err(|source| PackError::InvalidUtf8 {
        path: path.to_owned(),
        source,
    })
}

fn read_manifest(path: &Path) -> Result<PackManifest, PackError> {
    let manifest_path = path.join(MANIFEST_FILE);
    let source = read_bounded_text(&manifest_path, MAX_MANIFEST_BYTES)?;
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

fn read_snippet_configs(path: &Path) -> Result<(Vec<(PathBuf, Config)>, usize), PackError> {
    let snippets_path = path.join(SNIPPETS_DIR);
    let snippets_metadata = fs::symlink_metadata(&snippets_path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PackError::MissingSnippets
        } else {
            PackError::Read {
                path: snippets_path.clone(),
                source,
            }
        }
    })?;
    if !snippets_metadata.file_type().is_dir() {
        return Err(PackError::MissingSnippets);
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&snippets_path).map_err(|source| PackError::Read {
        path: snippets_path.clone(),
        source,
    })? {
        if files.len() >= MAX_SNIPPET_FILES {
            return Err(PackError::TooManyFiles {
                maximum: MAX_SNIPPET_FILES,
            });
        }
        let entry = entry.map_err(|source| PackError::Read {
            path: snippets_path.clone(),
            source,
        })?;
        files.push(entry.path());
    }
    files.sort();
    let mut configs = Vec::new();
    let mut aggregate_bytes = 0usize;
    for file in files {
        let metadata = fs::symlink_metadata(&file).map_err(|source| PackError::Read {
            path: file.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(PackError::InvalidFile { path: file });
        }
        if file.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        if configs.len() >= MAX_SNIPPET_FILES {
            return Err(PackError::TooManyFiles {
                maximum: MAX_SNIPPET_FILES,
            });
        }
        aggregate_bytes = aggregate_bytes.checked_add(metadata.len() as usize).ok_or(
            PackError::PackTooLarge {
                maximum: MAX_PACK_BYTES,
            },
        )?;
        if aggregate_bytes > MAX_PACK_BYTES {
            return Err(PackError::PackTooLarge {
                maximum: MAX_PACK_BYTES,
            });
        }
        let source = read_bounded_text(&file, MAX_SNIPPET_FILE_BYTES)?;
        let config = toml::from_str(&source).map_err(|source| PackError::Snippet {
            path: file.clone(),
            source,
        })?;
        configs.push((file, config));
    }
    Ok((configs, aggregate_bytes))
}

fn load_pack(path: &Path) -> Result<(PackManifest, Vec<(PathBuf, Config)>), PackError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| PackError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.file_type().is_dir() {
        return Err(PackError::NotDirectory(path.to_owned()));
    }
    let manifest = read_manifest(path)?;
    let (configs, snippet_bytes) = read_snippet_configs(path)?;
    let manifest_bytes = fs::symlink_metadata(path.join(MANIFEST_FILE))
        .map(|metadata| metadata.len() as usize)
        .unwrap_or(usize::MAX);
    if manifest_bytes.saturating_add(snippet_bytes) > MAX_PACK_BYTES {
        return Err(PackError::PackTooLarge {
            maximum: MAX_PACK_BYTES,
        });
    }
    Ok((manifest, configs))
}

pub fn inspect_pack(path: impl AsRef<Path>) -> Result<PackInspection, PackError> {
    let path = path.as_ref();
    let (manifest, configs) = load_pack(path)?;
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
    let (manifest, configs) = load_pack(path)?;
    let inspection = PackInspection {
        manifest,
        snippet_files: configs.len(),
        expansion_count: configs
            .iter()
            .map(|(_, config)| config.expansion.len())
            .sum(),
        hotkey_count: configs.iter().map(|(_, config)| config.hotkey.len()).sum(),
        command_count: configs
            .iter()
            .map(|(_, config)| {
                config
                    .expansion
                    .iter()
                    .filter(|item| item.command.is_some())
                    .count()
                    + config.hotkey.len()
            })
            .sum(),
    };
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_pack_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("wayexpand-pack-{}-{nonce}", std::process::id()))
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

    #[test]
    fn oversized_manifest_is_rejected_before_parsing() {
        let path = test_pack_path();
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        fs::write(
            path.join(MANIFEST_FILE),
            format!(
                "format_version = 1\nid = \"demo\"\nversion = \"1\"\nname = \"{}\"\npublisher = \"Test\"\n",
                "x".repeat(MAX_MANIFEST_BYTES)
            ),
        )
        .unwrap();
        assert!(matches!(
            inspect_pack(&path),
            Err(PackError::FileTooLarge { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_snippet_is_rejected() {
        use std::os::unix::fs::symlink;

        let path = test_pack_path();
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        fs::write(
            path.join(MANIFEST_FILE),
            "format_version = 1\nid = \"demo\"\nversion = \"1\"\nname = \"Demo\"\npublisher = \"Test\"\n",
        )
        .unwrap();
        let target = path.join("outside.toml");
        fs::write(
            &target,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'y'\n",
        )
        .unwrap();
        symlink(&target, path.join(SNIPPETS_DIR).join("linked.toml")).unwrap();
        assert!(matches!(
            inspect_pack(&path),
            Err(PackError::InvalidFile { .. })
        ));
        fs::remove_dir_all(path).unwrap();
    }
}
