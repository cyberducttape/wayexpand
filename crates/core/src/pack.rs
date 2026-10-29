use crate::{CommandConfig, Config};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const MANIFEST_FILE: &str = "wayexpand-pack.toml";
/// OpenSSH signature over [`pack_digest`], made with `wayexpand pack sign`.
pub const SIGNATURE_FILE: &str = "wayexpand-pack.sig";
/// `ssh-keygen -Y` namespace for pack signatures.
const SIGNATURE_NAMESPACE: &str = "wayexpand-pack";
const SNIPPETS_DIR: &str = "snippets";
const SIGNATURE_TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SIGNATURE_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
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
    /// Human review record for curated content. This is descriptive metadata,
    /// not a cryptographic trust signal; signatures authenticate the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<PackReview>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackReview {
    pub reviewed_by: String,
    /// Calendar date in `YYYY-MM-DD` form.
    pub reviewed_at: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackInspection {
    pub manifest: PackManifest,
    pub snippet_files: usize,
    pub expansion_count: usize,
    pub hotkey_count: usize,
    pub command_count: usize,
    /// Read-only content and action declarations for trust review. Callers
    /// should present these as untrusted pack data, never execute them.
    pub expansions: Vec<PackExpansionReview>,
    pub hotkeys: Vec<PackHotkeyReview>,
    /// SHA-256 fingerprint of the canonical manifest and snippet-file digest.
    pub fingerprint: String,
    /// Capabilities the snippets actually use.
    pub required_capabilities: Vec<String>,
    /// Whether a signature file is present (not whether it verifies).
    pub signed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackExpansionReview {
    pub file: String,
    pub id: String,
    pub trigger: String,
    pub description: String,
    pub replacement: String,
    pub command: Option<PackCommandReview>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackHotkeyReview {
    pub file: String,
    pub chord: String,
    pub description: String,
    pub command: PackCommandReview,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackCommandReview {
    pub action: Option<String>,
    pub program: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub environment: crate::CommandEnvironment,
    pub pass_env: Vec<String>,
}

fn review_command(command: &CommandConfig) -> PackCommandReview {
    PackCommandReview {
        action: command.action.clone(),
        program: command.program.clone(),
        args: command.args.clone(),
        timeout_ms: command.timeout_ms,
        environment: command.environment,
        pass_env: command.pass_env.clone(),
    }
}

fn review_content(
    configs: &[(PathBuf, Config)],
) -> (Vec<PackExpansionReview>, Vec<PackHotkeyReview>) {
    let mut expansions = Vec::new();
    let mut hotkeys = Vec::new();
    for (file, config) in configs {
        let file = file.to_string_lossy().into_owned();
        for expansion in &config.expansion {
            expansions.push(PackExpansionReview {
                file: file.clone(),
                id: expansion.id.clone(),
                trigger: expansion.trigger.clone(),
                description: expansion.description.clone(),
                replacement: expansion.replacement.clone(),
                command: expansion.command.as_ref().map(review_command),
            });
        }
        for hotkey in &config.hotkey {
            hotkeys.push(PackHotkeyReview {
                file: file.clone(),
                chord: hotkey.chord.clone(),
                description: hotkey.description.clone(),
                command: review_command(&hotkey.command),
            });
        }
    }
    (expansions, hotkeys)
}

#[derive(Debug, Error)]
pub enum PackError {
    #[error("pack path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("pack manifest is invalid: {0}")]
    Manifest(#[from] toml::de::Error),
    #[error("pack contents changed while they were being inspected")]
    ChangedDuringInspection,
    #[error("pack format version {0} is unsupported")]
    UnsupportedFormat(u32),
    #[error("pack manifest field `{0}` is empty")]
    EmptyField(&'static str),
    #[error("pack id `{0}` contains characters unsafe for a local directory name")]
    InvalidId(String),
    #[error("pack has no snippets directory")]
    MissingSnippets,
    #[error("pack file operation failed for {path}: {source}")]
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
    #[error("pack manifest has invalid semantic version `{0}`")]
    InvalidVersion(String),
    #[error("pack review date must be a real date in YYYY-MM-DD form")]
    InvalidReviewDate,
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
    if !is_valid_pack_id(&manifest.id) {
        return Err(PackError::InvalidId(manifest.id));
    }
    semver::Version::parse(&manifest.version)
        .map_err(|_| PackError::InvalidVersion(manifest.version.clone()))?;
    if let Some(review) = &manifest.review {
        for (value, field) in [
            (review.reviewed_by.as_str(), "review.reviewed_by"),
            (review.reviewed_at.as_str(), "review.reviewed_at"),
            (review.summary.as_str(), "review.summary"),
        ] {
            if value.trim().is_empty() {
                return Err(PackError::EmptyField(field));
            }
        }
        if !valid_review_date(&review.reviewed_at) {
            return Err(PackError::InvalidReviewDate);
        }
    }
    if let Some(required) = &manifest.min_wayexpand_version {
        semver::Version::parse(required)
            .map_err(|_| PackError::InvalidVersion(required.clone()))?;
        if version_is_newer(required, env!("CARGO_PKG_VERSION")) {
            return Err(PackError::RequiresNewerVersion {
                required: required.clone(),
            });
        }
    }
    Ok(manifest)
}

/// Whether `id` is safe as one local directory name: 1-128 ASCII letters,
/// digits, '.', '_' or '-', starting with a letter or digit. The leading
/// character rule rejects `.` and `..`, which would name the pack directory
/// itself or its parent, and hidden entries such as installer lock files.
pub fn is_valid_pack_id(id: &str) -> bool {
    id.len() <= 128
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_review_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    if !digits(0..4) || !digits(5..7) || !digits(8..10) {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or_default();
    let month = value[5..7].parse::<u32>().unwrap_or_default();
    let day = value[8..10].parse::<u32>().unwrap_or_default();
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let maximum = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=maximum).contains(&day)
}

/// Whether dotted version `required` is newer than `current`.
fn version_is_newer(required: &str, current: &str) -> bool {
    let Ok(required) = semver::Version::parse(required) else {
        return true;
    };
    let Ok(current) = semver::Version::parse(current) else {
        return true;
    };
    required > current
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

struct StablePack {
    manifest: PackManifest,
    configs: Vec<(PathBuf, Config)>,
    fingerprint: String,
}

fn load_pack_stable(path: &Path) -> Result<StablePack, PackError> {
    let before = pack_fingerprint(path)?;
    let (manifest, configs) = load_pack(path)?;
    let after = pack_fingerprint(path)?;
    if before != after {
        return Err(PackError::ChangedDuringInspection);
    }
    Ok(StablePack {
        manifest,
        configs,
        fingerprint: after,
    })
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
    let path = path.as_ref();
    let files = [MANIFEST_FILE];
    let snippets = path.join(SNIPPETS_DIR);
    let mut snippet_files = Vec::new();
    for entry in fs::read_dir(&snippets).map_err(|source| PackError::Read {
        path: snippets.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| PackError::Read {
            path: snippets.clone(),
            source,
        })?;
        let name = entry.file_name();
        if Path::new(&name)
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            snippet_files.push((name, entry.path()));
        }
    }
    snippet_files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = String::from("wayexpand-pack-digest-v1\n");
    for file in files {
        append_pack_digest_file(&mut digest, &path.join(file), file)?;
    }
    for (name, file_path) in snippet_files {
        let name = match name.to_str() {
            Some(name) => format!("{SNIPPETS_DIR}/{name}"),
            // A NUL cannot occur in a filename, making this byte-exact label
            // unambiguous with every valid UTF-8 filename. Avoid lossy
            // conversion, which can map distinct names to the same label.
            None => format!(
                "{SNIPPETS_DIR}/\0{}",
                name.as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
        };
        append_pack_digest_file(&mut digest, &file_path, &name)?;
    }
    Ok(digest)
}

/// Compact stable fingerprint of every file covered by the pack signature.
pub fn pack_fingerprint(path: impl AsRef<Path>) -> Result<String, PackError> {
    use sha2::{Digest, Sha256};
    let digest = pack_digest(path)?;
    Ok(Sha256::digest(digest.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn append_pack_digest_file(digest: &mut String, path: &Path, label: &str) -> Result<(), PackError> {
    use sha2::{Digest, Sha256};

    let text = read_bounded_text(path, MAX_SNIPPET_FILE_BYTES)?;
    let hash = Sha256::digest(text.as_bytes());
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    digest.push_str(&format!("{hex}  {label}\n"));
    Ok(())
}

#[cfg(test)]
fn read_signature_tool_output<R: Read>(mut reader: R) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take((MAX_SIGNATURE_TOOL_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > MAX_SIGNATURE_TOOL_OUTPUT_BYTES {
        return Err(io::Error::other(
            "ssh-keygen output exceeded the safety limit",
        ));
    }
    Ok(output)
}

fn set_nonblocking<R: AsRawFd>(stream: &R) -> io::Result<()> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain_signature_tool_output<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..64 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.len() > MAX_SIGNATURE_TOOL_OUTPUT_BYTES {
                    return Err(io::Error::other(
                        "ssh-keygen output exceeded the safety limit",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn write_signature_tool_input<W: Write + AsRawFd>(
    writer: &mut W,
    input: &[u8],
    deadline: Instant,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    set_nonblocking(writer)?;
    let mut written = 0;
    while written < input.len() {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "ssh-keygen input write timed out",
            ));
        }
        match writer.write(&input[written..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "ssh-keygen stdin closed",
                ))
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn wait_for_signature_tool_writer(
    result: &mpsc::Receiver<io::Result<()>>,
    timeout: Duration,
) -> Result<(), PackError> {
    match result.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(PackError::SignatureTool(error.to_string())),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(PackError::SignatureTool(
            "ssh-keygen stdin writer did not finish after process exit".into(),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(PackError::SignatureTool(
            "ssh-keygen stdin writer stopped without reporting a result".into(),
        )),
    }
}

fn run_ssh_keygen(args: &[&str], input: &str) -> Result<Output, PackError> {
    let mut command = Command::new("ssh-keygen");
    configure_process_group(&mut command);
    let mut child = command
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PackError::SignatureTool("ssh-keygen stdout was not captured".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| PackError::SignatureTool("ssh-keygen stderr was not captured".into()))?;
    let mut supervisor = ChildSupervisor::new(child);
    if let Err(error) = set_nonblocking(&stdout).and_then(|_| set_nonblocking(&stderr)) {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return Err(PackError::SignatureTool(error.to_string()));
    }
    let deadline = Instant::now() + SIGNATURE_TOOL_TIMEOUT;
    let input = input.as_bytes().to_owned();
    let stdin_cancelled = Arc::new(AtomicBool::new(false));
    let writer_cancelled = Arc::clone(&stdin_cancelled);
    let (writer_result_sender, writer_result_receiver) = mpsc::sync_channel(1);
    let stdin_writer = match thread::Builder::new()
        .name("wayexpand-pack-signature-stdin".into())
        .spawn(move || {
            let result = match stdin {
                Some(mut stdin) => {
                    write_signature_tool_input(&mut stdin, &input, deadline, &writer_cancelled)
                }
                None => Ok(()),
            };
            let _ = writer_result_sender.send(result);
        }) {
        Ok(writer) => writer,
        Err(error) => {
            supervisor.kill_group();
            let _ = supervisor.reap();
            return Err(PackError::SignatureTool(format!(
                "could not start ssh-keygen stdin writer: {error}"
            )));
        }
    };
    // The completion channel is the synchronization point. Dropping the join
    // handle avoids making cancellation depend on the OS scheduling this
    // short-lived writer thread.
    drop(stdin_writer);
    let mut stdout = stdout;
    let mut stderr = stderr;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = drain_signature_tool_output(&mut stdout, &mut stdout_bytes)
        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
    let mut stderr_eof = drain_signature_tool_output(&mut stderr, &mut stderr_bytes)
        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
    let status = loop {
        if !stdout_eof {
            stdout_eof = drain_signature_tool_output(&mut stdout, &mut stdout_bytes)
                .map_err(|error| PackError::SignatureTool(error.to_string()))?;
        }
        if !stderr_eof {
            stderr_eof = drain_signature_tool_output(&mut stderr, &mut stderr_bytes)
                .map_err(|error| PackError::SignatureTool(error.to_string()))?;
        }
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                let status = supervisor
                    .reap()
                    .map_err(|error| PackError::SignatureTool(error.to_string()))?;
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                while !(stdout_eof && stderr_eof) && Instant::now() < drain_deadline {
                    stdout_eof = drain_signature_tool_output(&mut stdout, &mut stdout_bytes)
                        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
                    stderr_eof = drain_signature_tool_output(&mut stderr, &mut stderr_bytes)
                        .map_err(|error| PackError::SignatureTool(error.to_string()))?;
                    if !(stdout_eof && stderr_eof) {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                if !stdout_eof || !stderr_eof {
                    return Err(PackError::SignatureTool(
                        "ssh-keygen output did not complete".into(),
                    ));
                }
                break status;
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                stdin_cancelled.store(true, Ordering::Release);
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err(PackError::SignatureTool(
                    "ssh-keygen timed out or could not be monitored".into(),
                ));
            }
        }
    };
    stdin_cancelled.store(true, Ordering::Release);
    wait_for_signature_tool_writer(&writer_result_receiver, Duration::from_millis(100))?;
    Ok(Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

fn write_signature_atomic(path: &Path, contents: &[u8]) -> Result<(), PackError> {
    write_signature_atomic_with_sync(path, contents, |directory| directory.sync_all())
}

fn write_signature_atomic_with_sync(
    path: &Path,
    contents: &[u8],
    sync_directory: impl FnOnce(&fs::File) -> io::Result<()>,
) -> Result<(), PackError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(PackError::Read {
                    path: path.to_owned(),
                    source: io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "signature target must be a regular, non-symlink file",
                    ),
                });
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PackError::Read {
                path: path.to_owned(),
                source,
            });
        }
    }
    let parent_directory = fs::File::open(parent).map_err(|source| PackError::Read {
        path: parent.to_owned(),
        source,
    })?;
    if !parent_directory
        .metadata()
        .map_err(|source| PackError::Read {
            path: parent.to_owned(),
            source,
        })?
        .is_dir()
    {
        return Err(PackError::Read {
            path: parent.to_owned(),
            source: io::Error::new(
                io::ErrorKind::NotADirectory,
                "signature parent is not a directory",
            ),
        });
    }

    for attempt in 0..16 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(
            ".{}.tmp.{}.{}.{}",
            SIGNATURE_FILE,
            std::process::id(),
            nonce,
            attempt
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(PackError::Read {
                    path: temporary,
                    source,
                });
            }
        };
        let committed = (|| -> Result<(), PackError> {
            file.write_all(contents).map_err(|source| PackError::Read {
                path: temporary.clone(),
                source,
            })?;
            file.sync_all().map_err(|source| PackError::Read {
                path: temporary.clone(),
                source,
            })?;
            fs::rename(&temporary, path).map_err(|source| PackError::Read {
                path: path.to_owned(),
                source,
            })?;
            Ok(())
        })();
        if committed.is_err() {
            let _ = fs::remove_file(&temporary);
            return committed;
        }
        return sync_directory(&parent_directory).map_err(|error| PackError::Read {
                path: path.to_owned(),
                source: io::Error::new(
                    error.kind(),
                    format!(
                        "signature was replaced, but its parent directory could not be synced; durability is uncertain: {error}"
                    ),
                ),
        });
    }
    Err(PackError::Read {
        path: path.to_owned(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique signature temporary file",
        ),
    })
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
    write_signature_atomic(&signature_path, &output.stdout)
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
    let StablePack {
        manifest,
        configs,
        fingerprint,
    } = load_pack_stable(path)?;
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
    let (expansions, hotkeys) = review_content(&configs);
    Ok(PackInspection {
        required_capabilities: required_capabilities(&configs).into_iter().collect(),
        signed: path.join(SIGNATURE_FILE).is_file(),
        manifest,
        snippet_files: configs.len(),
        expansion_count,
        hotkey_count,
        command_count,
        expansions,
        hotkeys,
        fingerprint,
    })
}

/// Load a pack for review. Commands are intentionally stripped from the
/// returned configuration; importing a pack must never grant execution
/// privileges implicitly.
pub fn import_pack(path: impl AsRef<Path>) -> Result<(PackInspection, Config, usize), PackError> {
    let path = path.as_ref();
    let StablePack {
        manifest,
        configs,
        fingerprint,
    } = load_pack_stable(path)?;
    let (expansions, hotkeys) = review_content(&configs);
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
        expansions,
        hotkeys,
        fingerprint,
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_PACK_ID: AtomicU64 = AtomicU64::new(0);

    fn test_pack_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique = TEST_PACK_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "wayexpand-pack-{}-{nonce}-{unique}",
            std::process::id()
        ))
    }

    fn write_review_pack(path: &Path, replacement: &str) {
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        fs::write(
            path.join(MANIFEST_FILE),
            "format_version=1\nid=\"review.test\"\nversion=\"1.0.0\"\nname=\"Review test\"\npublisher=\"Tests\"\n",
        )
        .unwrap();
        fs::write(
            path.join(SNIPPETS_DIR).join("main.toml"),
            format!("[[expansion]]\ntrigger=\";review\"\nreplacement={replacement:?}\n"),
        )
        .unwrap();
    }

    #[test]
    fn pack_review_fingerprint_tracks_manifest_and_snippet_content() {
        let path = test_pack_path();
        write_review_pack(&path, "first version");
        let first = inspect_pack(&path).unwrap();
        assert_eq!(first.fingerprint.len(), 64);
        assert_eq!(first.expansions[0].replacement, "first version");

        write_review_pack(&path, "second version");
        let second = inspect_pack(&path).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
        assert_eq!(second.expansions[0].replacement, "second version");
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn signature_tool_output_is_bounded() {
        assert_eq!(
            read_signature_tool_output(&b"ssh output"[..]).unwrap(),
            b"ssh output"
        );
        assert!(
            read_signature_tool_output(&vec![b'x'; MAX_SIGNATURE_TOOL_OUTPUT_BYTES + 1][..])
                .is_err()
        );
    }

    #[test]
    fn signature_tool_writer_wait_is_bounded() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let started = Instant::now();
        let error = wait_for_signature_tool_writer(&receiver, Duration::from_millis(10))
            .expect_err("a writer that does not report completion must time out");

        assert!(
            matches!(error, PackError::SignatureTool(message) if message.contains("did not finish"))
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn signing_refuses_a_symlink_target_and_atomically_replaces_regular_signature() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = test_pack_path();
        fs::create_dir_all(&directory).unwrap();
        let signature = directory.join(SIGNATURE_FILE);
        let target = directory.join("unrelated-file");
        fs::write(&target, b"preserve me").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, &signature).unwrap();

        let error = write_signature_atomic(&signature, b"signature")
            .expect_err("a symlink must not be followed when signing");
        assert!(matches!(
            error,
            PackError::Read { source, .. }
                if source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read(&target).unwrap(), b"preserve me");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );

        fs::remove_file(&signature).unwrap();
        fs::write(&signature, b"old signature").unwrap();
        write_signature_atomic(&signature, b"new signature").unwrap();
        assert_eq!(fs::read(&signature).unwrap(), b"new signature");
        assert_eq!(
            fs::metadata(&signature).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn signature_directory_sync_failure_reports_that_replacement_happened() {
        let directory = test_pack_path();
        fs::create_dir_all(&directory).unwrap();
        let signature = directory.join(SIGNATURE_FILE);
        fs::write(&signature, b"old signature").unwrap();

        let error = write_signature_atomic_with_sync(&signature, b"new signature", |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap_err();

        assert!(matches!(
            &error,
            PackError::Read { source, .. }
                if source.to_string().contains("signature was replaced")
                    && source.to_string().contains("durability is uncertain")
        ));
        assert_eq!(fs::read(&signature).unwrap(), b"new signature");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pack_digest_keeps_distinct_non_utf8_filenames_distinct() {
        use std::os::unix::ffi::OsStrExt;

        let path = test_pack_path();
        let snippets = path.join(SNIPPETS_DIR);
        fs::create_dir_all(&snippets).unwrap();
        fs::write(path.join(MANIFEST_FILE), "manifest").unwrap();
        let first = std::ffi::OsStr::from_bytes(b"entry-\xfe.toml");
        let second = std::ffi::OsStr::from_bytes(b"entry-\xff.toml");
        fs::write(snippets.join(first), "first").unwrap();
        fs::write(snippets.join(second), "second").unwrap();

        let digest = pack_digest(&path).unwrap();

        let labels = digest
            .lines()
            .filter_map(|line| line.split_once("  ").map(|(_, label)| label))
            .filter(|label| label.starts_with("snippets/"))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 2);
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.as_bytes().contains(&0))
                .count(),
            2
        );
        assert_ne!(labels[0], labels[1]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pack_digest_preserves_the_existing_utf8_signature_format() {
        use sha2::{Digest, Sha256};

        let path = test_pack_path();
        let snippets = path.join(SNIPPETS_DIR);
        fs::create_dir_all(&snippets).unwrap();
        fs::write(path.join(MANIFEST_FILE), "manifest").unwrap();
        fs::write(snippets.join("main.toml"), "snippet").unwrap();

        let hash = |bytes: &[u8]| {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let expected = format!(
            "wayexpand-pack-digest-v1\n{}  wayexpand-pack.toml\n{}  snippets/main.toml\n",
            hash(b"manifest"),
            hash(b"snippet")
        );
        assert_eq!(pack_digest(&path).unwrap(), expected);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn signature_output_drain_is_bounded_for_continuous_output() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "yes ssh-keygen"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn continuous-output child");
        let mut stdout = child.stdout.take().expect("child stdout");
        set_nonblocking(&stdout).expect("make stdout nonblocking");
        let mut output = Vec::new();
        let started = Instant::now();
        let result = drain_signature_tool_output(&mut stdout, &mut output)
            .expect("bounded drain should yield before the total cap");
        assert!(!result);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "continuous output exceeded the drain budget: {:?}",
            started.elapsed()
        );
        let _ = child.kill();
        let _ = child.wait();
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
                "format_version = 1\nid = \"demo\"\nversion = \"1.0.0\"\nname = \"{}\"\npublisher = \"Test\"\n",
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

    #[test]
    fn invalid_minimum_version_is_rejected() {
        let path = test_pack_path();
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        fs::write(
            path.join(MANIFEST_FILE),
            "format_version = 1\nid = \"demo\"\nversion = \"1.0.0\"\nname = \"Demo\"\npublisher = \"Test\"\nmin_wayexpand_version = \"future\"\n",
        )
        .unwrap();
        assert!(matches!(
            inspect_pack(&path),
            Err(PackError::InvalidVersion(version)) if version == "future"
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pack_version_id_and_review_date_are_strictly_validated() {
        let path = test_pack_path();
        fs::create_dir_all(path.join(SNIPPETS_DIR)).unwrap();
        let manifest = |version: &str, id: &str, date: &str| {
            format!(
                "format_version = 1\nid = {id:?}\nversion = {version:?}\nname = \"Reviewed\"\npublisher = \"WayExpand\"\n\n[review]\nreviewed_by = \"Maintainers\"\nreviewed_at = {date:?}\nsummary = \"Static insert-only templates\"\n"
            )
        };
        fs::write(
            path.join(MANIFEST_FILE),
            manifest("future", "demo", "2026-10-27"),
        )
        .unwrap();
        assert!(
            matches!(inspect_pack(&path), Err(PackError::InvalidVersion(value)) if value == "future")
        );

        fs::write(
            path.join(MANIFEST_FILE),
            manifest("1.0.0", "../escape", "2026-10-27"),
        )
        .unwrap();
        assert!(matches!(inspect_pack(&path), Err(PackError::InvalidId(_))));
        for id in ["..", ".", ".hidden", "-flag"] {
            fs::write(
                path.join(MANIFEST_FILE),
                manifest("1.0.0", id, "2026-10-27"),
            )
            .unwrap();
            assert!(
                matches!(inspect_pack(&path), Err(PackError::InvalidId(_))),
                "{id:?} must not be accepted as a pack id"
            );
        }

        fs::write(
            path.join(MANIFEST_FILE),
            manifest("1.0.0", "demo", "2025-02-29"),
        )
        .unwrap();
        assert!(matches!(
            inspect_pack(&path),
            Err(PackError::InvalidReviewDate)
        ));

        fs::write(
            path.join(MANIFEST_FILE),
            manifest("1.0.0", "demo", "2024-02-29"),
        )
        .unwrap();
        assert_eq!(
            inspect_pack(&path)
                .unwrap()
                .manifest
                .review
                .unwrap()
                .reviewed_at,
            "2024-02-29"
        );
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
            "format_version = 1\nid = \"demo\"\nversion = \"1.0.0\"\nname = \"Demo\"\npublisher = \"Test\"\n",
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
