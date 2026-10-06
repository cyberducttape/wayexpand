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
/// OpenSSH signature over [`pack_digest`], made with `wayexpand pack sign`.
pub const SIGNATURE_FILE: &str = "wayexpand-pack.sig";
/// `ssh-keygen -Y` namespace for pack signatures.
const SIGNATURE_NAMESPACE: &str = "wayexpand-pack";
const SNIPPETS_DIR: &str = "snippets";
use crate::limits::{
    MAX_PACK_BYTES, MAX_PACK_MANIFEST_BYTES as MAX_MANIFEST_BYTES,
    MAX_PACK_SNIPPET_FILES as MAX_SNIPPET_FILES,
    MAX_PACK_SNIPPET_FILE_BYTES as MAX_SNIPPET_FILE_BYTES,
};

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
    /// Oldest WayExpand version that understands this pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_wayexpand_version: Option<String>,
    /// Capabilities the pack's snippets use: any of `commands`,
    /// `broker_actions`, `clipboard`, `env`, and `forms`. When present, the
    /// pack may use nothing it does not declare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    /// Action Broker actions the pack may call (with `broker_actions`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackInspection {
    pub manifest: PackManifest,
    pub snippet_files: usize,
    pub expansion_count: usize,
    pub hotkey_count: usize,
    pub command_count: usize,
    /// Capabilities the snippets actually use.
    pub required_capabilities: Vec<String>,
    /// Whether a signature file is present (not whether it verifies).
    pub signed: bool,
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
    #[error("pack requires WayExpand {required} or newer")]
    RequiresNewerVersion { required: String },
    #[error("pack uses the `{0}` capability without declaring it")]
    UndeclaredCapability(String),
    #[error("pack calls Action Broker action `{0}` that is not in allowed_actions")]
    UndeclaredAction(String),
    #[error("pack is not signed")]
    Unsigned,
    #[error("pack signature is not valid for any trusted signer: {0}")]
    BadSignature(String),
    #[error("could not run ssh-keygen for pack signatures: {0}")]
    SignatureTool(String),
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
    if let Some(required) = &manifest.min_wayexpand_version {
        if version_is_newer(required, env!("CARGO_PKG_VERSION")) {
            return Err(PackError::RequiresNewerVersion {
                required: required.clone(),
            });
        }
    }
    Ok(manifest)
}

/// Whether dotted version `required` is newer than `current`.
fn version_is_newer(required: &str, current: &str) -> bool {
    let parse = |version: &str| -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    parse(required) > parse(current)
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
    check_declared_capabilities(&manifest, &configs)?;
    Ok((manifest, configs))
}

/// The capabilities a set of snippet files uses.
fn required_capabilities(configs: &[(PathBuf, Config)]) -> std::collections::BTreeSet<String> {
    let mut required = std::collections::BTreeSet::new();
    for (_, config) in configs {
        let commands = config
            .expansion
            .iter()
            .filter_map(|expansion| expansion.command.as_ref())
            .chain(config.hotkey.iter().map(|hotkey| &hotkey.command));
        for command in commands {
            required.insert(if command.action.is_some() {
                "broker_actions".to_owned()
            } else {
                "commands".to_owned()
            });
        }
        for expansion in &config.expansion {
            for name in crate::template_variables(&expansion.replacement) {
                if name == "clipboard" {
                    required.insert("clipboard".to_owned());
                } else if name.starts_with("env:") {
                    required.insert("env".to_owned());
                } else if ["field:", "prompt:", "choice:"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                {
                    required.insert("forms".to_owned());
                }
            }
        }
    }
    required
}

fn check_declared_capabilities(
    manifest: &PackManifest,
    configs: &[(PathBuf, Config)],
) -> Result<(), PackError> {
    let Some(declared) = &manifest.capabilities else {
        return Ok(());
    };
    for capability in required_capabilities(configs) {
        if !declared.contains(&capability) {
            return Err(PackError::UndeclaredCapability(capability));
        }
    }
    for (_, config) in configs {
        let actions = config
            .expansion
            .iter()
            .filter_map(|expansion| expansion.command.as_ref())
            .chain(config.hotkey.iter().map(|hotkey| &hotkey.command))
            .filter_map(|command| command.action.as_ref());
        for action in actions {
            if !manifest.allowed_actions.contains(action) {
                return Err(PackError::UndeclaredAction(action.clone()));
            }
        }
    }
    Ok(())
}

/// The canonical text a pack signature covers: the format tag, then the
/// SHA-256 of the manifest and of every snippet file, sorted by path.
/// Any change to any signed file changes the digest.
pub fn pack_digest(path: impl AsRef<Path>) -> Result<String, PackError> {
    use sha2::{Digest, Sha256};
    let path = path.as_ref();
    let mut files = vec![MANIFEST_FILE.to_owned()];
    let snippets = path.join(SNIPPETS_DIR);
    let mut snippet_names: Vec<String> = fs::read_dir(&snippets)
        .map_err(|source| PackError::Read {
            path: snippets.clone(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".toml"))
        .collect();
    snippet_names.sort();
    files.extend(
        snippet_names
            .into_iter()
            .map(|name| format!("{SNIPPETS_DIR}/{name}")),
    );
    let mut digest = String::from("wayexpand-pack-digest-v1\n");
    for file in files {
        let text = read_bounded_text(&path.join(&file), MAX_SNIPPET_FILE_BYTES)?;
        let hash = Sha256::digest(text.as_bytes());
        let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
        digest.push_str(&format!("{hex}  {file}\n"));
    }
    Ok(digest)
}

fn run_ssh_keygen(args: &[&str], input: &str) -> Result<std::process::Output, PackError> {
    use std::io::Write;
    let mut child = std::process::Command::new("ssh-keygen")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| PackError::SignatureTool(error.to_string()))?;
    }
    child
        .wait_with_output()
        .map_err(|error| PackError::SignatureTool(error.to_string()))
}

/// Sign a pack with an SSH private key, writing [`SIGNATURE_FILE`].
pub fn sign_pack(path: impl AsRef<Path>, key: &Path) -> Result<(), PackError> {
    let path = path.as_ref();
    load_pack(path)?;
    let digest = pack_digest(path)?;
    let key = key.to_string_lossy();
    let output = run_ssh_keygen(
        &["-Y", "sign", "-q", "-f", &key, "-n", SIGNATURE_NAMESPACE],
        &digest,
    )?;
    if !output.status.success() {
        return Err(PackError::SignatureTool(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let signature_path = path.join(SIGNATURE_FILE);
    fs::write(&signature_path, &output.stdout).map_err(|source| PackError::Read {
        path: signature_path,
        source,
    })
}

/// Check that an administrator signers file can be trusted: a regular file
/// owned by root and not writable by group or others. Otherwise anyone able
/// to edit it could make their own key trusted.
pub fn trusted_signers_file(path: &Path) -> Result<(), PackError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|source| PackError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(PackError::BadSignature(format!(
            "signers file {} must be a root-owned file not writable by others",
            path.display()
        )));
    }
    Ok(())
}

/// Verify a pack's signature against an OpenSSH `allowed_signers` file.
/// Returns the signer identity (principal).
pub fn verify_pack_signature(
    path: impl AsRef<Path>,
    allowed_signers: &Path,
) -> Result<String, PackError> {
    let path = path.as_ref();
    let signature_path = path.join(SIGNATURE_FILE);
    if !signature_path.is_file() {
        return Err(PackError::Unsigned);
    }
    let digest = pack_digest(path)?;
    let signature = signature_path.to_string_lossy();
    let signers = allowed_signers.to_string_lossy();
    let principals = run_ssh_keygen(
        &["-Y", "find-principals", "-s", &signature, "-f", &signers],
        "",
    )?;
    let principal = String::from_utf8_lossy(&principals.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|principal| !principal.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| PackError::BadSignature("no trusted signer matches".into()))?;
    let output = run_ssh_keygen(
        &[
            "-Y",
            "verify",
            "-f",
            &signers,
            "-I",
            &principal,
            "-n",
            SIGNATURE_NAMESPACE,
            "-s",
            &signature,
        ],
        &digest,
    )?;
    if !output.status.success() {
        return Err(PackError::BadSignature(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(principal)
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
        required_capabilities: required_capabilities(&configs).into_iter().collect(),
        signed: path.join(SIGNATURE_FILE).is_file(),
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
        required_capabilities: required_capabilities(&configs).into_iter().collect(),
        signed: path.join(SIGNATURE_FILE).is_file(),
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
