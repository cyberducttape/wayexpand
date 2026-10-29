//! `wayexpand sync`: optional Git synchronization of the snippet library.
//!
//! The configuration directory becomes a Git repository that tracks only the
//! portable library (`expansions.toml` and `snippets.d/*.toml`); portal
//! tokens, usage statistics, GUI preferences, and broker configuration are
//! never committed. Any Git remote works, or none. Nothing runs
//! automatically: each sync validates the local library, commits it, rebases
//! onto the remote, re-validates, and only then pushes. A merge that would
//! leave an invalid library is rolled back to the local version.

use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use wayexpand_core::Config;
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

/// Track only the library; everything else in the directory stays local.
const GITIGNORE: &str = "\
# Managed by `wayexpand sync`: only the snippet library is synchronized.
*
!.gitignore
!expansions.toml
!snippets.d/
!snippets.d/*.toml
";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_GIT_OUTPUT_BYTES: usize = 128 * 1024;
const SYNC_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// Serialize the complete sync transaction, not just individual Git
/// commands. Git's index lock does not cover fetch/rebase/validation/reset as
/// one unit, so two callers could otherwise interleave and publish the wrong
/// repository state.
struct SyncLock(File);

impl SyncLock {
    fn acquire(directory: &Path) -> Result<Self> {
        let path = directory.join(".wayexpand-sync.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .with_context(|| format!("could not open sync lock {}", path.display()))?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
        {
            bail!("sync lock has an unsafe type, owner, or link count");
        }
        // Tighten permissions through the already-verified descriptor rather
        // than chmod'ing a path that could be swapped for a symlink.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
            bail!("sync lock permissions could not be made private");
        }
        let deadline = Instant::now() + SYNC_LOCK_TIMEOUT;
        loop {
            // SAFETY: flock only operates on this owned lock descriptor.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Self(file));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(error).context("could not acquire sync lock");
            }
            if Instant::now() >= deadline {
                bail!(
                    "another WayExpand synchronization is already running in {}",
                    directory.display()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        // Closing the descriptor also releases the lock; explicitly unlocking
        // makes the ownership contract clear and keeps the operation harmless
        // if the implementation later retains the file for diagnostics.
        // SAFETY: the descriptor belongs to this lock instance.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SyncReport {
    pub(crate) directory: PathBuf,
    pub(crate) committed: bool,
    pub(crate) pulled: bool,
    pub(crate) pushed: bool,
    pub(crate) remote: Option<String>,
}

/// Variables that point git at a repository, index, object store, or
/// configuration other than the one `-C` names (`git rev-parse
/// --local-env-vars`). Git exports several of them while it runs hooks, so
/// a sync started from a hook, or from a shell with `GIT_DIR` set, would
/// otherwise commit and push in that other repository.
const GIT_LOCAL_ENVIRONMENT: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// `git -C directory args...` that can only act on `directory`'s
/// repository and never prompts on the terminal.
fn git_command(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    for variable in GIT_LOCAL_ENVIRONMENT {
        command.env_remove(variable);
    }
    command
        .arg("-C")
        .arg(directory)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    git_with_config(directory, args, false)
}

/// Run Git without system or global config. Repository intake must not execute
/// user configured clean/smudge filters while checking out untrusted content.
pub(crate) fn git_library_output(directory: &Path, args: &[&str]) -> Result<Output> {
    git_with_config(directory, args, true)
}

pub(crate) fn git_library_ok(directory: &Path, args: &[&str]) -> Result<String> {
    let output = git_library_output(directory, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            redact_git_diagnostic(&args.join(" ")),
            redact_git_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git_with_config(directory: &Path, args: &[&str], isolated_config: bool) -> Result<Output> {
    let mut stdout = Vec::new();
    let (status, stderr) = run_git(directory, args, None, isolated_config, &mut |chunk| {
        stdout.extend_from_slice(chunk);
        if stdout.len() > MAX_GIT_OUTPUT_BYTES {
            bail!("git output exceeded the safety limit");
        }
        Ok(())
    })?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Run git, handing its stdout to `on_stdout` as it arrives (so a listing
/// can be checked without holding all of it) and optionally feeding it
/// `stdin`. stderr is diagnostic only and is capped and returned.
fn run_git(
    directory: &Path,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    isolated_config: bool,
    on_stdout: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut command = git_command(directory, args);
    if isolated_config {
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_SSH")
            .env_remove("GIT_SSH_COMMAND")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS");
    }
    configure_process_group(&mut command);
    let mut child = command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git; is it installed?")?;
    let mut stdout = child.stdout.take().context("git stdout was not captured")?;
    let mut stderr = child.stderr.take().context("git stderr was not captured")?;
    let mut stdin_pipe = child.stdin.take();
    let mut supervisor = ChildSupervisor::new(child);
    if let Err(error) = set_nonblocking(&stdout)
        .and_then(|_| set_nonblocking(&stderr))
        .and_then(|_| {
            if let Some(pipe) = stdin_pipe.as_ref() {
                set_nonblocking(pipe)?;
            }
            Ok(())
        })
    {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return Err(error);
    }
    let mut stdin_offset = 0;
    let mut git_input = GitInput {
        pipe: &mut stdin_pipe,
        bytes: stdin.as_deref(),
        offset: &mut stdin_offset,
    };
    let result = supervise_git(
        &mut supervisor,
        &mut stdout,
        &mut stderr,
        &mut git_input,
        args,
        on_stdout,
    );
    if result.is_err() {
        supervisor.kill_group();
        let _ = supervisor.reap();
    }
    result
}

struct GitInput<'a> {
    pipe: &'a mut Option<ChildStdin>,
    bytes: Option<&'a [u8]>,
    offset: &'a mut usize,
}

fn supervise_git(
    supervisor: &mut ChildSupervisor,
    stdout: &mut std::process::ChildStdout,
    stderr: &mut std::process::ChildStderr,
    stdin: &mut GitInput<'_>,
    args: &[&str],
    on_stdout: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut stderr_bytes = Vec::new();
    let mut drain = |stdout: &mut std::process::ChildStdout,
                     stderr: &mut std::process::ChildStderr,
                     stderr_bytes: &mut Vec<u8>|
     -> Result<(bool, bool)> {
        let stdout_eof = drain_git_output(stdout, &mut *on_stdout)?;
        let stderr_eof = drain_git_output(stderr, &mut |chunk| {
            stderr_bytes.extend_from_slice(chunk);
            if stderr_bytes.len() > MAX_GIT_OUTPUT_BYTES {
                bail!("git output exceeded the safety limit");
            }
            Ok(())
        })?;
        Ok((stdout_eof, stderr_eof))
    };
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        if let Some(input) = stdin.bytes {
            write_git_input(stdin.pipe, input, stdin.offset)?;
        }
        let (stdout_eof, stderr_eof) = drain(stdout, stderr, &mut stderr_bytes)?;
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                let status = supervisor.reap().context("could not reap git")?;
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                let mut stdout_eof = stdout_eof;
                let mut stderr_eof = stderr_eof;
                while !(stdout_eof && stderr_eof) && Instant::now() < drain_deadline {
                    (stdout_eof, stderr_eof) = drain(stdout, stderr, &mut stderr_bytes)?;
                    if !(stdout_eof && stderr_eof) {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                // Callers parse stdout (tree listings, outgoing paths) for
                // safety decisions, so a truncated listing must fail closed.
                // stderr is diagnostic only and may legitimately stay open in
                // a persistent SSH control master outside git's group.
                if !stdout_eof {
                    bail!(
                        "git {} output did not complete",
                        redact_git_diagnostic(&args.join(" "))
                    );
                }
                return Ok((status, stderr_bytes));
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => bail!(
                "git {} timed out or could not be monitored",
                redact_git_diagnostic(&args.join(" "))
            ),
        }
    }
}

/// Write only a bounded amount of Git stdin per supervision turn. The pipe is
/// nonblocking, so a Git child that stops reading cannot pin sync past its
/// deadline, and no writer thread needs an unbounded join during cleanup.
fn write_git_input(pipe: &mut Option<ChildStdin>, input: &[u8], offset: &mut usize) -> Result<()> {
    const WRITES_PER_TURN: usize = 64;
    for _ in 0..WRITES_PER_TURN {
        if *offset >= input.len() {
            pipe.take();
            return Ok(());
        }
        let Some(result) = pipe
            .as_mut()
            .map(|stdin| std::io::Write::write(stdin, &input[*offset..]))
        else {
            return Ok(());
        };
        match result {
            Ok(0) => {
                pipe.take();
                return Ok(());
            }
            Ok(written) => *offset += written,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                pipe.take();
                return Ok(());
            }
            Err(error) => return Err(error).context("could not write git stdin"),
        }
    }
    if *offset >= input.len() {
        pipe.take();
    }
    Ok(())
}

/// Bounds for a NUL-separated Git listing, read record by record.
#[derive(Clone, Copy)]
struct RecordLimits {
    records: usize,
    record_bytes: usize,
}

/// One record per library file: the primary configuration, `.gitignore`, and
/// at most one `snippets.d` layer per snippet the configuration model allows.
const MAX_LIBRARY_FILES: usize = wayexpand_core::limits::MAX_EXPANSIONS + 2;
/// A path up to PATH_MAX plus an `ls-tree` header (mode, type, and a SHA-256
/// object name).
const MAX_GIT_RECORD_BYTES: usize = 4096 + 128;
const LIBRARY_LISTING: RecordLimits = RecordLimits {
    records: MAX_LIBRARY_FILES,
    record_bytes: MAX_GIT_RECORD_BYTES,
};
/// Outgoing history repeats paths once per commit that touches them (and
/// `status` may list files outside the library), so these are bounded far
/// more loosely; only the distinct foreign paths are kept.
/// Foreign paths named in a refusal; the first few are enough to act on.
const MAX_REPORTED_FOREIGN_PATHS: usize = 32;
const OUTGOING_HISTORY_LISTING: RecordLimits = RecordLimits {
    records: 4_000_000,
    record_bytes: MAX_GIT_RECORD_BYTES,
};

/// Run a git command whose stdout is NUL-terminated records (`-z`) and hand
/// each record to `on_record` as it is read, so a large library is never
/// held, or refused, as one block of output. Exceeding `limits` fails
/// closed, as does a failed command or a record that is not UTF-8.
fn git_records(
    directory: &Path,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    limits: RecordLimits,
    mut on_record: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut pending = Vec::new();
    let mut records = 0_usize;
    let too_long = || {
        anyhow::anyhow!(
            "git reported a path longer than {} bytes",
            limits.record_bytes
        )
    };
    let mut accept = |record: &[u8]| -> Result<()> {
        if record.len() > limits.record_bytes {
            return Err(too_long());
        }
        records += 1;
        if records > limits.records {
            bail!(
                "git {} listed more than {} entries (a snippet library has at most {} files)",
                redact_git_diagnostic(&args.join(" ")),
                limits.records,
                MAX_LIBRARY_FILES
            );
        }
        on_record(std::str::from_utf8(record).context("git reported a non-UTF-8 path")?)
    };
    let (status, stderr) = run_git(directory, args, stdin, false, &mut |chunk| {
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|&byte| byte == 0) {
            if pending.is_empty() {
                accept(&rest[..end])?;
            } else {
                pending.extend_from_slice(&rest[..end]);
                accept(&pending)?;
                pending.clear();
            }
            rest = &rest[end + 1..];
        }
        pending.extend_from_slice(rest);
        if pending.len() > limits.record_bytes {
            return Err(too_long());
        }
        Ok(())
    })?;
    if !status.success() {
        bail!(
            "git {} failed: {}",
            redact_git_diagnostic(&args.join(" ")),
            redact_git_diagnostic(String::from_utf8_lossy(&stderr).trim())
        );
    }
    // `-z` terminates every record, so anything left is a truncated one.
    if !pending.is_empty() {
        bail!(
            "git {} output ended inside an entry",
            redact_git_diagnostic(&args.join(" "))
        );
    }
    Ok(())
}

fn set_nonblocking<R: AsRawFd>(stream: &R) -> Result<()> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        bail!(
            "could not make git output nonblocking: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn drain_git_output<R: Read>(
    reader: &mut R,
    output: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<bool> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..64 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => output(&buffer[..count])?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

#[cfg(test)]
fn read_git_output<R: Read>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take((MAX_GIT_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > MAX_GIT_OUTPUT_BYTES {
        return Err(std::io::Error::other(
            "git output exceeded the safety limit",
        ));
    }
    Ok(output)
}

fn git_ok(directory: &Path, args: &[&str]) -> Result<String> {
    let output = git(directory, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            redact_git_diagnostic(&args.join(" ")),
            redact_git_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn validate_library(config_path: &Path) -> Result<()> {
    let primary = library_primary_name(config_path)?;
    if matches!(
        primary,
        ".gitignore" | ".git" | ".wayexpand-sync.lock" | "snippets.d"
    ) {
        bail!("library primary filename conflicts with a reserved library path");
    }
    if !std::fs::symlink_metadata(config_path).is_ok_and(|metadata| metadata.file_type().is_file())
    {
        bail!("library is invalid: primary configuration is not a plain file");
    }
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?;
    let gitignore = directory.join(".gitignore");
    if let Ok(metadata) = std::fs::symlink_metadata(&gitignore) {
        if !metadata.file_type().is_file() {
            bail!("library is invalid: .gitignore is not a plain file");
        }
    }
    let snippets = directory.join("snippets.d");
    if let Ok(metadata) = std::fs::symlink_metadata(&snippets) {
        if !metadata.file_type().is_dir() {
            bail!("library is invalid: snippets.d is not a plain directory");
        }
    }
    for file in Config::layer_files(directory)
        .map_err(|error| anyhow::anyhow!("library is invalid: {}", error.safe_summary()))?
    {
        if !std::fs::symlink_metadata(&file).is_ok_and(|metadata| metadata.file_type().is_file()) {
            bail!(
                "library is invalid: {} is not a plain file; sync tracks only regular snippet files",
                file.display()
            );
        }
    }
    // Validate only after rejecting symlinks for every path Git may stage;
    // the regular fleet loader intentionally follows layer symlinks.
    Config::validate_library_files(config_path)
        .map_err(|error| anyhow::anyhow!("library is invalid: {}", error.safe_summary()))?;
    Ok(())
}

/// Synchronized libraries are static by default. Commands and hotkeys are
/// executable policy, not ordinary snippet data, so a remote repository must
/// not be able to introduce them without an explicit local approval.
fn validate_library_executable_policy(config_path: &Path, allow_executable: bool) -> Result<()> {
    if allow_executable {
        return Ok(());
    }
    let primary = Config::load(config_path)
        .map_err(|error| anyhow::anyhow!("could not inspect synced library: {error}"))?;
    reject_executable_content(config_path, &primary)?;
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?;
    for file in Config::layer_files(directory)
        .map_err(|error| anyhow::anyhow!("could not inspect synced layers: {error}"))?
    {
        let layer = Config::load(&file)
            .map_err(|error| anyhow::anyhow!("could not inspect synced layer: {error}"))?;
        reject_executable_content(&file, &layer)?;
    }
    Ok(())
}

fn reject_executable_content(path: &Path, config: &Config) -> Result<()> {
    if let Some(expansion) = config
        .expansion
        .iter()
        .find(|expansion| expansion.command.is_some())
    {
        bail!(
            "synced library {} contains command-backed expansion {:?}; executable content is disabled by default. Review the revision and retry with --allow-executable --approved-revision <full-commit>",
            path.display(),
            expansion.trigger
        );
    }
    if let Some(hotkey) = config.hotkey.first() {
        bail!(
            "synced library {} contains executable hotkey {:?}; executable content is disabled by default. Review the revision and retry with --allow-executable --approved-revision <full-commit>",
            path.display(),
            hotkey.chord
        );
    }
    Ok(())
}

fn validate_approved_revision(revision: Option<&str>, fetched: &str) -> Result<()> {
    let Some(revision) = revision else {
        bail!(
            "remote executable content requires --approved-revision with the full reviewed commit"
        );
    };
    if !(revision.len() == 40 || revision.len() == 64)
        || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("--approved-revision must be a full 40- or 64-character Git object ID");
    }
    if !revision.eq_ignore_ascii_case(fetched) {
        bail!(
            "remote executable content is not the approved revision (fetched {fetched}, approved {revision})"
        );
    }
    Ok(())
}

fn remote(directory: &Path) -> Result<Option<String>> {
    // `origin` is the explicitly configured synchronization remote.  Picking
    // the first entry from `git remote` makes a repository with a backup or
    // mirror remote sync unpredictably.
    let output = git(directory, &["remote", "get-url", "origin"])?;
    if output.status.success() {
        return Ok(Some(redact_remote_url(
            String::from_utf8_lossy(&output.stdout).trim(),
        )));
    }
    Ok(None)
}

/// Remove URL credentials before a remote is included in status or JSON.
fn redact_remote_url(value: &str) -> String {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return value.to_owned();
    };
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let safe_authority = authority
        .rfind('@')
        .map_or(authority, |at| &authority[at + 1..]);
    let tail = &remainder[authority_end..];
    let mut result = format!("{scheme}://{safe_authority}");
    if let Some((query, fragment)) = tail.split_once('#') {
        result.push_str(&redact_query(query));
        if !fragment.is_empty() {
            result.push_str("#[REDACTED]");
        }
    } else {
        result.push_str(&redact_query(tail));
    }
    result
}

fn redact_query(tail: &str) -> String {
    let Some((path, query)) = tail.split_once('?') else {
        return tail.to_owned();
    };
    if query.is_empty() {
        tail.to_owned()
    } else {
        format!("{path}?[REDACTED]")
    }
}

fn redact_git_diagnostic(value: &str) -> String {
    value
        .split_inclusive(char::is_whitespace)
        .map(|part| {
            let Some(scheme_marker) = part.find("://") else {
                return part.to_owned();
            };
            // Step past the whole delimiter: localized Git messages quote
            // URLs with multi-byte characters such as `»`.
            let scheme_start = part[..scheme_marker]
                .char_indices()
                .rfind(|&(_, character)| {
                    !character.is_ascii_alphanumeric()
                        && character != '+'
                        && character != '-'
                        && character != '.'
                })
                .map_or(0, |(index, character)| index + character.len_utf8());
            let (url, suffix) = part.split_at(part.trim_end_matches(char::is_whitespace).len());
            let (url, trailing) =
                url.split_at(url.trim_end_matches([',', ';', ')', ']', '\'']).len());
            format!(
                "{}{}{}{}",
                &part[..scheme_start],
                redact_remote_url(&url[scheme_start..]),
                trailing,
                suffix
            )
        })
        .collect()
}

pub(crate) fn redact_library_diagnostic(value: &str) -> String {
    redact_git_diagnostic(value)
}

/// Interpret `git ls-remote --exit-code`: 0 lists the branch and 2 means
/// the remote has no such branch. Anything else (authentication, DNS, an
/// unreachable host) is a failure, not an empty remote, and must not lead
/// sync to skip fetching and verifying the remote before pushing.
fn remote_branch_listed(listing: &Output) -> Result<bool> {
    match listing.status.code() {
        Some(0) => Ok(true),
        Some(2) => Ok(false),
        _ => bail!(
            "could not query the origin remote: {}",
            redact_git_diagnostic(String::from_utf8_lossy(&listing.stderr).trim())
        ),
    }
}

/// Make the configuration directory a library repository.
pub(crate) fn init(config_path: &Path, remote_url: Option<&str>) -> Result<PathBuf> {
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?
        .to_path_buf();
    let _lock = SyncLock::acquire(&directory)?;
    validate_library(config_path)?;
    if !directory.join(".git").exists() {
        git_ok(&directory, &["init", "--quiet"])?;
    }
    let gitignore = directory.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, GITIGNORE).context("could not write .gitignore")?;
    }
    if let Some(url) = remote_url {
        if remote(&directory)?.is_some() {
            git_ok(&directory, &["remote", "set-url", "origin", url])?;
        } else {
            git_ok(&directory, &["remote", "add", "origin", url])?;
        }
    }
    commit_library(
        &directory,
        library_primary_name(config_path)?,
        "Track WayExpand snippet library",
    )?;
    Ok(directory)
}

fn library_primary_name(config_path: &Path) -> Result<&str> {
    config_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("configuration filename must be valid UTF-8")
}

fn commit_library(directory: &Path, primary: &str, message: &str) -> Result<bool> {
    let primary_pathspec = format!(":(literal){primary}");
    // `-A` is essential here: if the final snippets.d file is deleted, the
    // directory disappears and therefore cannot be selected by an existence
    // check. Git still understands the pathspec and stages the deletion.
    let mut add = vec!["add", "-A", "--"];
    for path in [".gitignore", "snippets.d"] {
        let present = directory.join(path).exists();
        let tracked = !present
            && git_ok(directory, &["ls-files", "--", path]).is_ok_and(|output| !output.is_empty());
        if present || tracked {
            add.push(path);
        }
    }
    if add.len() > 3 {
        git_ok(directory, &add)?;
    }
    let primary_present = directory.join(primary).exists();
    let primary_tracked = !primary_present
        && git_ok(directory, &["ls-files", "--", &primary_pathspec])
            .is_ok_and(|output| !output.is_empty());
    if primary_present || primary_tracked {
        // The default ignore rule excludes the entire config directory. Force
        // only this exact literal primary path, never the snippet directory.
        git_ok(directory, &["add", "-A", "-f", "--", &primary_pathspec])?;
    }
    // The index may already hold unrelated staged files. Commit only the
    // exact library paths that changed, so nothing else can be published.
    let mut library_paths = Vec::new();
    git_records(
        directory,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "--",
            ".gitignore",
            &primary_pathspec,
            "snippets.d",
        ],
        None,
        LIBRARY_LISTING,
        |path| {
            if is_library_path(path, primary) {
                library_paths.extend_from_slice(b":(literal)");
                library_paths.extend_from_slice(path.as_bytes());
                library_paths.push(0);
            }
            Ok(())
        },
    )?;
    if library_paths.is_empty() {
        return Ok(false);
    }
    // The paths go through stdin: a large library's paths would not fit
    // the argument list.
    let commit = [
        "commit",
        "--quiet",
        "--only",
        "-m",
        message,
        "--pathspec-from-file=-",
        "--pathspec-file-nul",
    ];
    let output = run_git(directory, &commit, Some(library_paths), false, &mut |_| {
        Ok(())
    })
    .and_then(|(status, stderr)| {
        if status.success() {
            Ok(())
        } else {
            bail!(
                "git commit failed: {}",
                redact_git_diagnostic(String::from_utf8_lossy(&stderr).trim())
            )
        }
    });
    output.context(
        "could not commit; set git user.name and user.email (globally or in the library repository)",
    )?;
    Ok(true)
}

/// Paths `wayexpand sync` is allowed to commit and publish.
fn is_library_path(path: &str, primary: &str) -> bool {
    path == ".gitignore"
        || path == primary
        || path
            .strip_prefix("snippets.d/")
            .is_some_and(|name| !name.contains('/') && name.ends_with(".toml") && name != ".toml")
}

/// Refuse a tree unless every entry is an allowlisted library path stored as
/// an ordinary file blob (not a symlink, submodule, or other special mode).
fn verify_library_tree(directory: &Path, revision: &str, primary: &str) -> Result<()> {
    let mut foreign = Vec::new();
    git_records(
        directory,
        &["ls-tree", "-r", "-z", "--full-tree", revision],
        None,
        LIBRARY_LISTING,
        |entry| {
            if entry.is_empty() {
                return Ok(());
            }
            // `<mode> <type> <object>\t<path>`
            let (header, path) = entry
                .split_once('\t')
                .context("git ls-tree reported an unexpected entry")?;
            let mode = header.split(' ').next().unwrap_or_default();
            if (!matches!(mode, "100644" | "100755") || !is_library_path(path, primary))
                && foreign.len() < MAX_REPORTED_FOREIGN_PATHS
            {
                foreign.push(path.to_owned());
            }
            Ok(())
        },
    )?;
    if foreign.is_empty() {
        return Ok(());
    }
    foreign.sort_unstable();
    bail!(
        "refusing to sync: Git tree {revision} contains entries that are not plain snippet \
         library files ({}); your local library is unchanged",
        foreign.join(", ")
    )
}

/// Refuse to push history that touches anything outside the library, such as
/// files committed by hand or by an older release. `base` is the remote tip;
/// without one, the whole history would be published and is checked.
fn verify_outgoing_paths(directory: &Path, base: Option<&str>, primary: &str) -> Result<()> {
    let range = base.map_or_else(|| "HEAD".to_owned(), |base| format!("{base}..HEAD"));
    // With `-z` and an empty format, every path is NUL-terminated. Paths may
    // legally contain newlines, so splitting on them as well would let
    // `expansions.toml\nanything` pass as two allowlisted paths.
    let mut foreign = std::collections::BTreeSet::new();
    git_records(
        directory,
        &[
            "log",
            "--format=",
            "--name-only",
            "--no-renames",
            "-m",
            "-z",
            &range,
        ],
        None,
        OUTGOING_HISTORY_LISTING,
        |path| {
            if !path.is_empty()
                && !is_library_path(path, primary)
                && foreign.len() < MAX_REPORTED_FOREIGN_PATHS
            {
                foreign.insert(path.to_owned());
            }
            Ok(())
        },
    )?;
    if foreign.is_empty() {
        return Ok(());
    }
    let foreign: Vec<String> = foreign.into_iter().collect();
    bail!(
        "refusing to push: outgoing commits change files outside the snippet library ({}); \
         remove them from the history in {}",
        foreign.join(", "),
        directory.display()
    )
}

/// Commit local changes, rebase onto the remote, re-validate, and push.
#[cfg(test)]
pub(crate) fn sync(config_path: &Path) -> Result<SyncReport> {
    sync_with_policy(config_path, false, None)
}

/// Synchronize a library with an explicit executable-content approval policy.
pub(crate) fn sync_with_policy(
    config_path: &Path,
    allow_executable: bool,
    approved_revision: Option<&str>,
) -> Result<SyncReport> {
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?
        .to_path_buf();
    if !directory.join(".git").exists() {
        bail!("the library is not a Git repository yet; run `wayexpand sync init [--remote URL]`");
    }
    let _lock = SyncLock::acquire(&directory)?;
    validate_library(config_path)?;
    validate_library_executable_policy(config_path, allow_executable)?;
    let primary = library_primary_name(config_path)?;
    let committed = commit_library(&directory, primary, "Sync WayExpand library")?;
    let remote = remote(&directory)?;
    let mut report = SyncReport {
        directory: directory.clone(),
        committed,
        pulled: false,
        pushed: false,
        remote: remote.clone(),
    };
    let Some(_remote) = remote else {
        return Ok(report);
    };
    let before = git_ok(&directory, &["rev-parse", "HEAD"])?;
    let branch = git_ok(&directory, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let listing = git(
        &directory,
        &["ls-remote", "--exit-code", "--heads", "origin", &branch],
    )?;
    let remote_has_branch = remote_branch_listed(&listing)?;
    if remote_has_branch {
        // Fetch first and inspect the remote tree before anything from it is
        // checked out: a remote must not be able to place arbitrary files,
        // symlinks, or submodules in the configuration directory.
        git_ok(&directory, &["fetch", "--quiet", "origin", &branch])?;
        verify_library_tree(&directory, "FETCH_HEAD", primary)?;
        let fetched_revision = git_ok(&directory, &["rev-parse", "FETCH_HEAD"])?;
        let rebase = git(&directory, &["rebase", "--quiet", "FETCH_HEAD"])?;
        if !rebase.status.success() {
            let _ = git(&directory, &["rebase", "--abort"]);
            restrict_library_permissions(&directory, primary);
            bail!(
                "the remote library conflicts with local changes; your local library is unchanged. \
                 Resolve it with git in {}",
                directory.display()
            );
        }
        report.pulled = true;
        restrict_library_permissions(&directory, primary);
        if let Err(error) = validate_library(config_path)
            .and_then(|()| validate_library_executable_policy(config_path, allow_executable))
        {
            // Never leave the daemon a broken library: restore the local one.
            git_ok(&directory, &["reset", "--quiet", "--hard", &before])?;
            // The reset rewrites files with the umask; keep them loadable.
            restrict_library_permissions(&directory, primary);
            bail!("remote changes would make the {error:#}; kept the local library");
        }
        if allow_executable {
            // Approval is tied to the exact fetched object, never merely to a
            // branch name that can move between review and synchronization.
            let executable_present = has_executable_content(config_path)?;
            if executable_present {
                if let Err(error) = validate_approved_revision(approved_revision, &fetched_revision)
                {
                    git_ok(&directory, &["reset", "--quiet", "--hard", &before])?;
                    restrict_library_permissions(&directory, primary);
                    bail!("remote executable approval failed: {error:#}; kept the local library");
                }
            }
        }
    }
    verify_outgoing_paths(
        &directory,
        remote_has_branch.then_some("FETCH_HEAD"),
        primary,
    )?;
    // The path history check prevents unrelated commits from being published;
    // inspect the actual outgoing tip too, so symlinks and special Git modes
    // cannot hide behind an otherwise allowlisted filename.
    verify_library_tree(&directory, "HEAD", primary)?;
    git_ok(
        &directory,
        &["push", "--quiet", "--set-upstream", "origin", &branch],
    )?;
    report.pushed = true;
    Ok(report)
}

fn has_executable_content(config_path: &Path) -> Result<bool> {
    let primary = Config::load(config_path)
        .map_err(|error| anyhow::anyhow!("could not inspect synced library: {error}"))?;
    if primary
        .expansion
        .iter()
        .any(|expansion| expansion.command.is_some())
        || !primary.hotkey.is_empty()
    {
        return Ok(true);
    }
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?;
    for file in Config::layer_files(directory)
        .map_err(|error| anyhow::anyhow!("could not inspect synced layers: {error}"))?
    {
        let layer = Config::load(&file)
            .map_err(|error| anyhow::anyhow!("could not inspect synced layer: {error}"))?;
        if layer
            .expansion
            .iter()
            .any(|expansion| expansion.command.is_some())
            || !layer.hotkey.is_empty()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Git writes files with the local umask; the library must not be group- or
/// world-writable to load, so pulled files are made private.
fn restrict_library_permissions(directory: &Path, primary: &str) {
    use std::os::unix::fs::PermissionsExt;
    // Never follow a symlink: chmod through one changes its target, which
    // can be any file the user owns.
    let private = |path: &Path| {
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    };
    private(&directory.join(primary));
    let snippets = directory.join("snippets.d");
    if std::fs::symlink_metadata(&snippets).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        let _ = std::fs::set_permissions(&snippets, std::fs::Permissions::from_mode(0o700));
    }
    if let Ok(entries) = std::fs::read_dir(snippets) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                private(&entry.path());
            }
        }
    }
}

/// A short human summary of the repository state.
pub(crate) fn status(config_path: &Path) -> Result<String> {
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?;
    if !directory.join(".git").exists() {
        return Ok("not synchronized (run `wayexpand sync init [--remote URL]`)".into());
    }
    let remote = remote(directory)?.unwrap_or_else(|| "none".into());
    let mut changed = false;
    git_records(
        directory,
        &["status", "--porcelain", "-z"],
        None,
        OUTGOING_HISTORY_LISTING,
        |entry| {
            changed |= !entry.is_empty();
            Ok(())
        },
    )?;
    let last = git_ok(directory, &["log", "-1", "--format=%cr: %s"]).unwrap_or_default();
    Ok(format!(
        "repository: {}\nremote: {remote}\nlocal changes: {}\nlast commit: {last}",
        directory.display(),
        if changed { "yes" } else { "none" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "wayexpand-sync-test-{}-{suffix}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn prepare_repository() -> (PathBuf, PathBuf) {
        let directory = test_directory();
        std::fs::create_dir_all(&directory).expect("create test repository directory");
        std::fs::set_permissions(
            &directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("restrict test repository directory");
        let config_path = directory.join("expansions.toml");
        std::fs::write(
            &config_path,
            "[[expansion]]\ntrigger = \";test\"\nreplacement = \"ok\"\n",
        )
        .expect("write test configuration");
        std::fs::set_permissions(
            &config_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict test configuration");
        git_ok(&directory, &["init", "--quiet"]).expect("initialize test repository");
        git_ok(&directory, &["config", "user.name", "WayExpand Test"])
            .expect("configure test Git user name");
        git_ok(
            &directory,
            &["config", "user.email", "wayexpand-test@example.invalid"],
        )
        .expect("configure test Git user email");
        (directory, config_path)
    }

    #[test]
    fn synced_libraries_reject_commands_and_hotkeys_by_default() {
        let directory = test_directory();
        std::fs::create_dir_all(&directory).expect("create policy test directory");
        std::fs::set_permissions(
            &directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("restrict policy test directory");
        let config_path = directory.join("expansions.toml");
        std::fs::write(
            &config_path,
            "[[expansion]]\ntrigger = \";run\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"/bin/true\"\n",
        )
        .expect("write command-backed configuration");
        std::fs::set_permissions(
            &config_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict command-backed configuration");
        let error = validate_library_executable_policy(&config_path, false)
            .expect_err("commands must require explicit approval");
        assert!(error.to_string().contains("executable content is disabled"));

        std::fs::write(
            &config_path,
            "[[hotkey]]\nchord = \"Ctrl+Alt+W\"\n[hotkey.command]\nprogram = \"/bin/true\"\n",
        )
        .expect("write hotkey configuration");
        let error = validate_library_executable_policy(&config_path, false)
            .expect_err("hotkeys must require explicit approval");
        assert!(error.to_string().contains("executable hotkey"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn executable_approval_requires_the_exact_full_revision() {
        let fetched = "0123456789abcdef0123456789abcdef01234567";
        assert!(validate_approved_revision(Some(fetched), fetched).is_ok());
        assert!(validate_approved_revision(Some(&fetched[..39]), fetched).is_err());
        assert!(validate_approved_revision(Some(&"f".repeat(40)), fetched).is_err());
        assert!(validate_approved_revision(None, fetched).is_err());
    }

    #[test]
    fn outgoing_history_rejects_paths_with_embedded_newlines() {
        let (directory, _config_path) = prepare_repository();
        git_ok(&directory, &["add", "expansions.toml"]).unwrap();
        git_ok(&directory, &["commit", "--quiet", "-m", "library"]).unwrap();
        let hidden = directory.join("expansions.toml\nsnippets.d");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("private.toml"), "secret\n").unwrap();
        git_ok(&directory, &["add", "--all"]).unwrap();
        git_ok(&directory, &["commit", "--quiet", "-m", "smuggle"]).unwrap();
        std::fs::remove_dir_all(&hidden).unwrap();
        git_ok(&directory, &["add", "--all"]).unwrap();
        git_ok(&directory, &["commit", "--quiet", "-m", "hide"]).unwrap();

        // The file is gone from HEAD, so only the history check can catch it.
        verify_library_tree(&directory, "HEAD", "expansions.toml").unwrap();
        let error = verify_outgoing_paths(&directory, None, "expansions.toml").unwrap_err();
        assert!(
            error.to_string().contains("outside the snippet library"),
            "{error:#}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn git_output_is_bounded() {
        assert_eq!(read_git_output(&b"git output"[..]).unwrap(), b"git output");
        assert!(read_git_output(&vec![b'x'; MAX_GIT_OUTPUT_BYTES + 1][..]).is_err());
    }

    #[test]
    fn git_output_drain_yields_for_continuous_output() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "yes wayexpand"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn continuous-output child");
        let mut stdout = child.stdout.take().expect("child stdout");
        set_nonblocking(&stdout).expect("make stdout nonblocking");
        let mut output = Vec::new();
        let started = Instant::now();
        let _ = drain_git_output(&mut stdout, &mut |chunk| {
            output.extend_from_slice(chunk);
            Ok(())
        })
        .expect("drain output");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn git_stdin_writer_yields_when_a_child_stops_reading() {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("10")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command
            .spawn()
            .expect("start a child that does not read stdin");
        let mut pipe = Some(child.stdin.take().expect("capture stdin"));
        let mut supervisor = ChildSupervisor::new(child);
        set_nonblocking(pipe.as_ref().unwrap()).expect("make test pipe nonblocking");
        let input = vec![b'x'; 1024 * 1024];
        let mut offset = 0;
        let started = Instant::now();

        write_git_input(&mut pipe, &input, &mut offset).expect("nonblocking write yields");

        assert!(
            offset < input.len(),
            "the unconsumed input should fill the pipe"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(pipe);
        supervisor.kill_group();
        let _ = supervisor.reap();
    }

    #[test]
    fn sync_uses_a_generic_commit_message() {
        let (directory, config_path) = prepare_repository();
        sync(&config_path).expect("sync local library");
        let message =
            git_ok(&directory, &["log", "-1", "--format=%s"]).expect("read commit message");
        assert_eq!(message, "Sync WayExpand library");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn validation_includes_snippet_layers() {
        let (directory, config_path) = prepare_repository();
        let snippets = directory.join("snippets.d");
        std::fs::create_dir(&snippets).expect("create snippets directory");
        std::fs::set_permissions(
            &snippets,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("restrict snippets directory");
        let invalid = snippets.join("broken.toml");
        std::fs::write(&invalid, "this is not valid toml = [").expect("write invalid snippet");
        std::fs::set_permissions(
            &invalid,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict invalid snippet");
        assert!(validate_library(&config_path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn validation_rejects_duplicate_base_and_layer_entries() {
        let (directory, config_path) = prepare_repository();
        std::fs::write(
            &config_path,
            "[[expansion]]\ntrigger = \";duplicate\"\nreplacement = \"base\"\n",
        )
        .expect("write base snippet");
        std::fs::set_permissions(
            &config_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict base snippet");
        let snippets = directory.join("snippets.d");
        std::fs::create_dir(&snippets).expect("create snippets directory");
        std::fs::set_permissions(
            &snippets,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("restrict snippets directory");
        let duplicate = snippets.join("duplicate.toml");
        std::fs::write(
            &duplicate,
            "[[expansion]]\ntrigger = \";duplicate\"\nreplacement = \"layer\"\n",
        )
        .expect("write duplicate layer");
        std::fs::set_permissions(
            &duplicate,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict duplicate layer");

        assert!(validate_library(&config_path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_validation_refuses_symlinked_layers() {
        use std::os::unix::fs::PermissionsExt;
        let (directory, config_path) = prepare_repository();
        let snippets = directory.join("snippets.d");
        std::fs::create_dir(&snippets).expect("create snippets directory");
        std::fs::set_permissions(&snippets, std::fs::Permissions::from_mode(0o700))
            .expect("restrict snippets directory");
        let target = directory.join("outside.toml");
        std::fs::write(
            &target,
            "[[expansion]]\ntrigger = \";t\"\nreplacement = \"x\"\n",
        )
        .expect("write target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
            .expect("restrict target");
        std::os::unix::fs::symlink(&target, snippets.join("linked.toml")).expect("symlink layer");

        // The loader accepts the link; sync refuses to track it.
        Config::validate_layer_files(&config_path).expect("loader follows layer symlinks");
        let error = validate_library(&config_path).expect_err("sync tracks plain files only");
        assert!(format!("{error:#}").contains("linked.toml"), "{error:#}");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn deleting_last_snippet_is_staged_as_a_git_deletion() {
        let (directory, _config_path) = prepare_repository();
        let snippets = directory.join("snippets.d");
        std::fs::create_dir(&snippets).expect("create snippets directory");
        std::fs::set_permissions(
            &snippets,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("restrict snippets directory");
        let snippet = snippets.join("old.toml");
        std::fs::write(
            &snippet,
            "[[expansion]]\ntrigger = \";old\"\nreplacement = \"x\"\n",
        )
        .expect("write snippet");
        std::fs::set_permissions(
            &snippet,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .expect("restrict snippet");
        commit_library(&directory, "expansions.toml", "add snippet").expect("commit snippet");
        std::fs::remove_file(snippet).expect("delete snippet");
        std::fs::remove_dir(snippets).expect("delete empty snippets directory");
        assert!(
            commit_library(&directory, "expansions.toml", "remove snippet")
                .expect("commit deletion")
        );
        let tracked = git_ok(&directory, &["ls-tree", "-r", "--name-only", "HEAD"])
            .expect("list tracked files");
        assert!(!tracked.lines().any(|path| path == "snippets.d/old.toml"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn library_commit_excludes_unrelated_staged_files() {
        let (directory, config_path) = prepare_repository();
        // Bypass the managed .gitignore the way a user's own tooling might.
        std::fs::write(directory.join("private.txt"), "secret").expect("write private file");
        git_ok(&directory, &["add", "--force", "private.txt"]).expect("stage private file");

        init(&config_path, None).expect("initialize library");

        let committed = git_ok(&directory, &["show", "--name-only", "--format=", "HEAD"])
            .expect("list committed files");
        assert!(committed.lines().any(|path| path == "expansions.toml"));
        assert!(!committed.lines().any(|path| path == "private.txt"));
        let still_staged =
            git_ok(&directory, &["diff", "--cached", "--name-only"]).expect("list staged files");
        assert_eq!(still_staged, "private.txt");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_refuses_to_push_history_outside_the_library() {
        let (directory, config_path) = prepare_repository();
        let remote_directory = test_directory();
        std::fs::create_dir_all(&remote_directory).expect("create remote directory");
        git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
        let remote_url = remote_directory.display().to_string();
        init(&config_path, Some(&remote_url)).expect("initialize library");
        assert!(
            sync(&config_path)
                .expect("library-only history syncs")
                .pushed
        );
        let published = git_ok(&remote_directory, &["rev-parse", "HEAD"]).expect("remote tip");
        std::fs::write(directory.join("private.txt"), "secret").expect("write private file");
        git_ok(&directory, &["add", "--force", "private.txt"]).expect("stage private file");
        git_ok(&directory, &["commit", "--quiet", "-m", "manual"]).expect("manual commit");

        let error = sync(&config_path).expect_err("foreign history must not be pushed");
        assert!(format!("{error:#}").contains("private.txt"));
        assert_eq!(
            git_ok(&remote_directory, &["rev-parse", "HEAD"]).expect("remote tip"),
            published,
            "the foreign commit must not reach the remote"
        );
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(remote_directory);
    }

    /// Publish `files` (path, contents, symlink target) from a second clone so
    /// they arrive at `directory` through the remote.
    fn push_from_another_clone(remote_url: &str, files: &[(&str, &str, bool)]) {
        let clone = test_directory();
        let output = Command::new("git")
            .args(["clone", "--quiet", remote_url])
            .arg(&clone)
            .output()
            .expect("clone remote");
        assert!(output.status.success());
        git_ok(&clone, &["config", "user.name", "Remote"]).expect("configure clone");
        git_ok(&clone, &["config", "user.email", "remote@example.invalid"])
            .expect("configure clone");
        for (path, contents, symlink) in files {
            let target = clone.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).expect("create parent");
            if *symlink {
                std::os::unix::fs::symlink(contents, &target).expect("create symlink");
            } else {
                std::fs::write(&target, contents).expect("write file");
            }
            git_ok(&clone, &["add", "--force", path]).expect("stage remote file");
        }
        git_ok(&clone, &["commit", "--quiet", "-m", "remote change"]).expect("commit");
        git_ok(&clone, &["push", "--quiet"]).expect("push remote change");
        let _ = std::fs::remove_dir_all(clone);
    }

    #[test]
    fn a_library_too_large_for_one_output_block_still_syncs() {
        let (directory, config_path) = prepare_repository();
        let remote_directory = test_directory();
        std::fs::create_dir_all(&remote_directory).expect("create remote directory");
        git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
        let remote_url = remote_directory.display().to_string();
        init(&config_path, Some(&remote_url)).expect("initialize library");
        // 2,000 layers with long names: the tree listing alone is several
        // times MAX_GIT_OUTPUT_BYTES, and the paths overflow nothing.
        let layers = 2_000;
        let snippets = directory.join("snippets.d");
        std::fs::create_dir_all(&snippets).expect("create snippets.d");
        std::fs::set_permissions(&snippets, PermissionsExt::from_mode(0o700))
            .expect("restrict snippets.d");
        for index in 0..layers {
            let layer = snippets.join(format!("{}-{index:04}.toml", "layer".repeat(30)));
            std::fs::write(
                &layer,
                format!("[[expansion]]\ntrigger = \";layer{index}\"\nreplacement = \"ok\"\n"),
            )
            .expect("write layer");
            std::fs::set_permissions(&layer, PermissionsExt::from_mode(0o600))
                .expect("restrict layer");
        }
        let report = sync(&config_path).expect("sync a large library");
        assert!(report.committed && report.pushed);
        let mut published = 0;
        git_records(
            &remote_directory,
            &["ls-tree", "-r", "-z", "--name-only", "HEAD"],
            None,
            LIBRARY_LISTING,
            |path| {
                published += usize::from(path.starts_with("snippets.d/"));
                Ok(())
            },
        )
        .expect("list the published tree");
        assert_eq!(published, layers);
        assert!(status(&config_path)
            .expect("status of a large library")
            .contains("local changes: none"));
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(remote_directory);
    }

    #[test]
    fn git_listings_beyond_their_limits_fail_closed() {
        let (directory, config_path) = prepare_repository();
        sync(&config_path).expect("sync local library");
        let listing = |limits| {
            git_records(
                &directory,
                &["ls-tree", "-r", "-z", "--full-tree", "HEAD"],
                None,
                limits,
                |_| Ok(()),
            )
        };
        let entries = RecordLimits {
            records: 0,
            record_bytes: MAX_GIT_RECORD_BYTES,
        };
        assert!(format!("{:#}", listing(entries).unwrap_err()).contains("more than 0 entries"));
        let long = RecordLimits {
            records: MAX_LIBRARY_FILES,
            record_bytes: 8,
        };
        assert!(format!("{:#}", listing(long).unwrap_err()).contains("longer than 8 bytes"));
        assert!(listing(LIBRARY_LISTING).is_ok());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_refuses_remote_symlinks_and_foreign_files_before_checkout() {
        for (path, contents, symlink) in [
            ("snippets.d/evil.toml", "/etc/passwd", true),
            ("autostart.sh", "#!/bin/sh\n", false),
        ] {
            let (directory, config_path) = prepare_repository();
            let remote_directory = test_directory();
            std::fs::create_dir_all(&remote_directory).expect("create remote directory");
            git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
            let remote_url = remote_directory.display().to_string();
            init(&config_path, Some(&remote_url)).expect("initialize library");
            assert!(sync(&config_path).expect("initial sync").pushed);
            push_from_another_clone(&remote_url, &[(path, contents, symlink)]);

            let error = sync(&config_path).expect_err("unsafe remote content is refused");
            assert!(format!("{error:#}").contains(path), "{error:#}");
            assert!(
                std::fs::symlink_metadata(directory.join(path)).is_err(),
                "{path} must not be checked out"
            );
            let _ = std::fs::remove_dir_all(directory);
            let _ = std::fs::remove_dir_all(remote_directory);
        }
    }

    #[test]
    fn sync_rebases_onto_a_remote_library_change() {
        let (directory, config_path) = prepare_repository();
        let remote_directory = test_directory();
        std::fs::create_dir_all(&remote_directory).expect("create remote directory");
        git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
        let remote_url = remote_directory.display().to_string();
        init(&config_path, Some(&remote_url)).expect("initialize library");
        assert!(sync(&config_path).expect("initial sync").pushed);
        push_from_another_clone(
            &remote_url,
            &[(
                "snippets.d/team.toml",
                "[[expansion]]\ntrigger = \";team\"\nreplacement = \"ok\"\n",
                false,
            )],
        );

        let report = sync(&config_path).expect("library-only remote change syncs");

        assert!(report.pulled && report.pushed);
        assert!(directory.join("snippets.d/team.toml").is_file());
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(remote_directory);
    }

    #[test]
    fn custom_primary_permissions_are_repaired_after_remote_rebase() {
        use std::os::unix::fs::PermissionsExt;

        let (directory, default_path) = prepare_repository();
        let primary = "team library.toml";
        let config_path = directory.join(primary);
        std::fs::rename(default_path, &config_path).expect("rename primary configuration");
        let remote_directory = test_directory();
        std::fs::create_dir_all(&remote_directory).expect("create remote directory");
        git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
        let remote_url = remote_directory.display().to_string();
        init(&config_path, Some(&remote_url)).expect("initialize custom primary library");
        assert!(sync(&config_path).expect("initial sync").pushed);
        push_from_another_clone(
            &remote_url,
            &[(
                primary,
                "[[expansion]]\ntrigger = \":remote\"\nreplacement = \"updated\"\n",
                false,
            )],
        );

        let report = sync(&config_path).expect("custom-primary remote update is valid");

        assert!(report.pulled && report.pushed);
        assert_eq!(
            std::fs::metadata(&config_path)
                .expect("stat pulled primary")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        Config::load(&config_path).expect("pulled primary remains loadable");
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(remote_directory);
    }

    #[test]
    fn permission_repair_never_follows_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let directory = test_directory();
        std::fs::create_dir_all(directory.join("snippets.d")).expect("create snippets.d");
        let script = directory.join("script.sh");
        std::fs::write(&script, "#!/bin/sh\n").expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make script executable");
        std::os::unix::fs::symlink(&script, directory.join("snippets.d/from-remote.toml"))
            .expect("create symlink");
        std::os::unix::fs::symlink(&script, directory.join("custom-primary.toml"))
            .expect("create custom primary symlink");

        restrict_library_permissions(&directory, "expansions.toml");
        restrict_library_permissions(&directory, "custom-primary.toml");

        let mode = std::fs::metadata(&script)
            .expect("stat script")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn library_path_allowlist_is_exact() {
        assert!(is_library_path("expansions.toml", "expansions.toml"));
        assert!(is_library_path(".gitignore", "expansions.toml"));
        assert!(is_library_path("snippets.d/work.toml", "expansions.toml"));
        assert!(!is_library_path(
            "snippets.d/nested/work.toml",
            "expansions.toml"
        ));
        assert!(!is_library_path("snippets.d/run.sh", "expansions.toml"));
        assert!(!is_library_path("portal-token", "expansions.toml"));
        assert!(!is_library_path("expansions.toml.bak", "expansions.toml"));
        assert!(is_library_path("team library.toml", "team library.toml"));
        assert!(!is_library_path("other.toml", "team library.toml"));
    }

    #[test]
    fn custom_primary_filename_is_staged_and_primary_symlinks_are_rejected() {
        let (directory, config_path) = prepare_repository();
        let custom_path = directory.join("team library.toml");
        std::fs::rename(&config_path, &custom_path).expect("rename primary config");
        init(&custom_path, None).expect("initialize custom primary library");
        assert_eq!(
            git_ok(&directory, &["ls-files", "--", "team library.toml"])
                .expect("list custom primary"),
            "team library.toml"
        );
        assert!(git_ok(&directory, &["ls-files", "--", "expansions.toml"])
            .expect("list canonical primary")
            .is_empty());

        let real_path = directory.join("real-config.toml");
        std::fs::rename(&custom_path, &real_path).expect("move primary behind symlink");
        std::os::unix::fs::symlink(&real_path, &custom_path).expect("symlink primary config");
        assert!(validate_library(&custom_path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_rejects_symlinked_library_directory_and_gitignore() {
        let (directory, config_path) = prepare_repository();
        let target_directory = directory.join("snippets-target");
        std::fs::create_dir(&target_directory).expect("create snippets target");
        std::os::unix::fs::symlink(&target_directory, directory.join("snippets.d"))
            .expect("symlink snippets directory");
        assert!(validate_library(&config_path).is_err());
        std::fs::remove_file(directory.join("snippets.d")).expect("remove snippets symlink");

        let target_file = directory.join("ignore-target");
        std::fs::write(&target_file, "*").expect("create ignore target");
        std::os::unix::fs::symlink(&target_file, directory.join(".gitignore"))
            .expect("symlink gitignore");
        assert!(validate_library(&config_path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn custom_primary_filename_syncs_to_and_from_the_remote() {
        let (directory, default_path) = prepare_repository();
        let custom_path = directory.join("workstation-library.toml");
        std::fs::rename(&default_path, &custom_path).expect("rename primary config");
        let remote_directory = test_directory();
        std::fs::create_dir_all(&remote_directory).expect("create remote directory");
        git_ok(&remote_directory, &["init", "--quiet", "--bare"]).expect("init bare remote");
        let remote_url = remote_directory.display().to_string();
        init(&custom_path, Some(&remote_url)).expect("initialize custom primary library");
        assert!(sync(&custom_path).expect("push custom primary").pushed);

        std::fs::write(
            &custom_path,
            "[[expansion]]\ntrigger = \":custom\"\nreplacement = \"updated\"\n",
        )
        .expect("update custom primary");
        assert!(
            sync(&custom_path)
                .expect("sync custom primary change")
                .pushed
        );
        let remote_tip =
            git_ok(&remote_directory, &["rev-parse", "HEAD"]).expect("read bare remote tip");
        assert!(
            git_ok(&directory, &["ls-tree", "-r", "--name-only", &remote_tip])
                .expect("list synced custom tree")
                .lines()
                .any(|path| path == "workstation-library.toml")
        );
        let _ = std::fs::remove_dir_all(directory);
        let _ = std::fs::remove_dir_all(remote_directory);
    }

    #[test]
    fn outgoing_tree_modes_reject_a_symlink_at_an_allowlisted_path() {
        let (directory, config_path) = prepare_repository();
        let target = directory.join("target");
        std::fs::write(&target, "not the config").expect("write symlink target");
        std::fs::remove_file(&config_path).expect("remove primary config");
        std::os::unix::fs::symlink(&target, &config_path).expect("replace primary with symlink");
        git_ok(&directory, &["add", "-f", "--", "expansions.toml"])
            .expect("stage malicious primary symlink");
        git_ok(&directory, &["commit", "--quiet", "-m", "malicious mode"])
            .expect("commit malicious primary symlink");
        let error = verify_library_tree(&directory, "HEAD", "expansions.toml")
            .expect_err("outgoing tree must reject symlink modes");
        assert!(format!("{error:#}").contains("expansions.toml"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_lock_refuses_symlinks_without_changing_the_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = test_directory();
        std::fs::create_dir_all(&directory).expect("create lock directory");
        let target = directory.join("unrelated-file");
        std::fs::write(&target, "leave this alone").expect("create target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
            .expect("make target readable");
        symlink(&target, directory.join(".wayexpand-sync.lock")).expect("create lock symlink");

        assert!(SyncLock::acquire(&directory).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"leave this alone");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sync_lock_is_private_and_regular() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_directory();
        std::fs::create_dir_all(&directory).expect("create lock directory");
        let lock = SyncLock::acquire(&directory).expect("acquire new sync lock");
        let metadata = lock.0.metadata().expect("stat lock descriptor");
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        drop(lock);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn remote_query_failures_are_errors_not_a_missing_branch() {
        use std::os::unix::process::ExitStatusExt;
        let listing = |code: i32, stderr: &str| Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        assert!(remote_branch_listed(&listing(0, "")).unwrap());
        assert!(!remote_branch_listed(&listing(2, "")).unwrap());
        let error = remote_branch_listed(&listing(
            128,
            "fatal: could not read from https://alice:topsecret@example.invalid/lib.git",
        ))
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("could not query the origin remote"),
            "{message}"
        );
        assert!(!message.contains("topsecret"), "{message}");
    }

    #[test]
    fn remote_reporting_redacts_credentials_and_diagnostic_urls() {
        assert_eq!(
            redact_remote_url("https://alice:topsecret@example.invalid/lib.git?token=private&ref=main#fragment-secret"),
            "https://example.invalid/lib.git?[REDACTED]#[REDACTED]"
        );
        assert_eq!(
            redact_remote_url("ssh://git@example.invalid/team/lib.git"),
            "ssh://example.invalid/team/lib.git"
        );
        // An unescaped `@` in a password must not leave part of it behind.
        assert_eq!(
            redact_remote_url("https://alice:p@ss@example.invalid/lib.git"),
            "https://example.invalid/lib.git"
        );
        // Query strings may contain arbitrary credential names, separators,
        // or encoded data. Status output redacts the whole query.
        assert_eq!(
            redact_remote_url(
                "https://store.example.invalid/lib.git?sv=2024&sig=abc%3D&X-Amz-Signature=def"
            ),
            "https://store.example.invalid/lib.git?[REDACTED]"
        );
        assert_eq!(
            redact_remote_url("https://store.example.invalid/lib.git?%74oken=private&ref=main"),
            "https://store.example.invalid/lib.git?[REDACTED]"
        );
        assert_eq!(
            redact_remote_url("https://store.example.invalid/lib.git?%ZZ=private&ref=main"),
            "https://store.example.invalid/lib.git?[REDACTED]"
        );
        assert_eq!(
            redact_remote_url("https://store.example.invalid/lib.git?custom=private;token=secret"),
            "https://store.example.invalid/lib.git?[REDACTED]"
        );
        let diagnostic = redact_git_diagnostic(
            "fatal: https://alice:topsecret@example.invalid/lib.git?password=hidden failed\n",
        );
        assert!(!diagnostic.contains("topsecret"));
        assert!(!diagnostic.contains("hidden"));
        assert!(diagnostic.contains("example.invalid/lib.git"));
        let localized = redact_git_diagnostic(
            "fatal: Konnte nicht von »https://alice:topsecret@example.invalid/lib.git« lesen\n",
        );
        assert!(!localized.contains("topsecret"));
        assert!(localized.contains("»https://example.invalid/lib.git«"));
    }

    #[test]
    fn origin_status_never_returns_embedded_credentials() {
        let (directory, config_path) = prepare_repository();
        git_ok(
            &directory,
            &[
                "remote",
                "add",
                "origin",
                "https://user:credential-secret@example.invalid/library.git?access_token=query-secret",
            ],
        )
        .expect("add credential-bearing origin");
        let shown = remote(&directory)
            .expect("read remote")
            .expect("origin exists");
        assert_eq!(shown, "https://example.invalid/library.git?[REDACTED]");
        let status = status(&config_path).expect("format status");
        assert!(!status.contains("credential-secret"));
        assert!(!status.contains("query-secret"));
        assert!(status.contains("example.invalid/library.git"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn init_adds_origin_when_another_remote_is_already_configured() {
        let (directory, config_path) = prepare_repository();
        git_ok(
            &directory,
            &[
                "remote",
                "add",
                "backup",
                "ssh://example.invalid/backup.git",
            ],
        )
        .expect("configure backup remote");

        init(&config_path, Some("ssh://example.invalid/origin.git"))
            .expect("configure the synchronization remote");

        assert_eq!(
            remote(&directory).expect("read origin remote"),
            Some("ssh://example.invalid/origin.git".into())
        );
        assert_eq!(
            git_ok(&directory, &["remote", "get-url", "backup"]).expect("read backup remote"),
            "ssh://example.invalid/backup.git"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn git_never_inherits_a_redirected_repository() {
        let command = git_command(Path::new("/library"), &["status"]);
        let envs: std::collections::HashMap<_, _> = command.get_envs().collect();
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(variable)),
                Some(&None),
                "{variable} must be removed"
            );
        }
        assert_eq!(
            envs.get(std::ffi::OsStr::new("GIT_TERMINAL_PROMPT")),
            Some(&Some(std::ffi::OsStr::new("0")))
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["-C", "/library", "status"]);
        // The list is git's own; keep it in step when git is available.
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--local-env-vars"])
            .output()
        {
            if output.status.success() {
                for variable in String::from_utf8_lossy(&output.stdout).lines() {
                    assert!(
                        GIT_LOCAL_ENVIRONMENT.contains(&variable),
                        "git reports {variable} as repository-local"
                    );
                }
            }
        }
    }
}
