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
    process::{Command, Output, Stdio},
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

fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    configure_process_group(&mut command);
    let mut child = command
        .arg("-C")
        .arg(directory)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git; is it installed?")?;
    let mut stdout = child.stdout.take().context("git stdout was not captured")?;
    let mut stderr = child.stderr.take().context("git stderr was not captured")?;
    let mut supervisor = ChildSupervisor::new(child);
    if let Err(error) = set_nonblocking(&stdout).and_then(|_| set_nonblocking(&stderr)) {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return Err(error);
    }
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        let stdout_eof = drain_git_output(&mut stdout, &mut stdout_bytes)?;
        let stderr_eof = drain_git_output(&mut stderr, &mut stderr_bytes)?;
        if stdout_bytes.len() > MAX_GIT_OUTPUT_BYTES || stderr_bytes.len() > MAX_GIT_OUTPUT_BYTES {
            supervisor.kill_group();
            let _ = supervisor.reap();
            bail!("git output exceeded the safety limit");
        }
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                let status = supervisor.reap().context("could not reap git")?;
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                let mut stdout_eof = stdout_eof;
                let mut stderr_eof = stderr_eof;
                while !(stdout_eof && stderr_eof) && Instant::now() < drain_deadline {
                    stdout_eof = drain_git_output(&mut stdout, &mut stdout_bytes)?;
                    stderr_eof = drain_git_output(&mut stderr, &mut stderr_bytes)?;
                    if stdout_bytes.len() > MAX_GIT_OUTPUT_BYTES
                        || stderr_bytes.len() > MAX_GIT_OUTPUT_BYTES
                    {
                        bail!("git output exceeded the safety limit");
                    }
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
                break status;
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                bail!(
                    "git {} timed out or could not be monitored",
                    redact_git_diagnostic(&args.join(" "))
                );
            }
        }
    };
    Ok(Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
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

fn drain_git_output<R: Read>(reader: &mut R, output: &mut Vec<u8>) -> Result<bool> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..64 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => output.extend_from_slice(&buffer[..count]),
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

/// Like `git_ok`, but keeps the output untrimmed for `-z` path lists.
fn git_ok_raw(directory: &Path, args: &[&str]) -> Result<String> {
    let output = git(directory, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            redact_git_diagnostic(&args.join(" ")),
            redact_git_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }
    String::from_utf8(output.stdout).context("git reported a non-UTF-8 path")
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
    let query = query
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key_lower = key.to_ascii_lowercase();
            if [
                "token",
                "password",
                "passwd",
                "secret",
                "auth",
                "credential",
                "api_key",
                "access_key",
                "key",
            ]
            .iter()
            .any(|sensitive| key_lower.contains(sensitive))
            {
                format!("{key}=[REDACTED]")
            } else if value.is_empty() {
                key.to_owned()
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{path}?{query}")
}

fn redact_git_diagnostic(value: &str) -> String {
    value
        .split_inclusive(char::is_whitespace)
        .map(|part| {
            let Some(scheme_marker) = part.find("://") else {
                return part.to_owned();
            };
            let scheme_start = part[..scheme_marker]
                .rfind(|character: char| {
                    !character.is_ascii_alphanumeric()
                        && character != '+'
                        && character != '-'
                        && character != '.'
                })
                .map_or(0, |index| index + 1);
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
    let staged = git_ok_raw(
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
    )?;
    let library_paths: Vec<String> = staged
        .split('\0')
        .filter(|path| is_library_path(path, primary))
        .map(|path| format!(":(literal){path}"))
        .collect();
    if library_paths.is_empty() {
        return Ok(false);
    }
    let mut commit = vec!["commit", "--quiet", "--only", "-m", message, "--"];
    commit.extend(library_paths.iter().map(String::as_str));
    git_ok(directory, &commit).context(
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
    let listing = git_ok_raw(directory, &["ls-tree", "-r", "-z", "--full-tree", revision])?;
    let mut foreign = Vec::new();
    for entry in listing.split('\0').filter(|entry| !entry.is_empty()) {
        // `<mode> <type> <object>\t<path>`
        let (header, path) = entry
            .split_once('\t')
            .context("git ls-tree reported an unexpected entry")?;
        let mode = header.split(' ').next().unwrap_or_default();
        if !matches!(mode, "100644" | "100755") || !is_library_path(path, primary) {
            foreign.push(path.to_owned());
        }
    }
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
    let touched = git_ok_raw(
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
    )?;
    let mut foreign: Vec<&str> = touched
        .split(['\0', '\n'])
        .filter(|path| !path.is_empty() && !is_library_path(path, primary))
        .collect();
    if foreign.is_empty() {
        return Ok(());
    }
    foreign.sort_unstable();
    foreign.dedup();
    bail!(
        "refusing to push: outgoing commits change files outside the snippet library ({}); \
         remove them from the history in {}",
        foreign.join(", "),
        directory.display()
    )
}

/// Commit local changes, rebase onto the remote, re-validate, and push.
pub(crate) fn sync(config_path: &Path) -> Result<SyncReport> {
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?
        .to_path_buf();
    if !directory.join(".git").exists() {
        bail!("the library is not a Git repository yet; run `wayexpand sync init [--remote URL]`");
    }
    let _lock = SyncLock::acquire(&directory)?;
    validate_library(config_path)?;
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
    let remote_has_branch = git(
        &directory,
        &["ls-remote", "--exit-code", "--heads", "origin", &branch],
    )?
    .status
    .success();
    if remote_has_branch {
        // Fetch first and inspect the remote tree before anything from it is
        // checked out: a remote must not be able to place arbitrary files,
        // symlinks, or submodules in the configuration directory.
        git_ok(&directory, &["fetch", "--quiet", "origin", &branch])?;
        verify_library_tree(&directory, "FETCH_HEAD", primary)?;
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
        if let Err(error) = validate_library(config_path) {
            // Never leave the daemon a broken library: restore the local one.
            git_ok(&directory, &["reset", "--quiet", "--hard", &before])?;
            // The reset rewrites files with the umask; keep them loadable.
            restrict_library_permissions(&directory, primary);
            bail!("remote changes would make the {error:#}; kept the local library");
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
    let changes = git_ok(directory, &["status", "--porcelain"])?;
    let last = git_ok(directory, &["log", "-1", "--format=%cr: %s"]).unwrap_or_default();
    Ok(format!(
        "repository: {}\nremote: {remote}\nlocal changes: {}\nlast commit: {last}",
        directory.display(),
        if changes.is_empty() { "none" } else { "yes" }
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
        let _ = drain_git_output(&mut stdout, &mut output).expect("drain output");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = child.kill();
        let _ = child.wait();
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
    fn remote_reporting_redacts_credentials_and_diagnostic_urls() {
        assert_eq!(
            redact_remote_url("https://alice:topsecret@example.invalid/lib.git?token=private&ref=main#fragment-secret"),
            "https://example.invalid/lib.git?token=[REDACTED]&ref=main#[REDACTED]"
        );
        assert_eq!(
            redact_remote_url("ssh://git@example.invalid/team/lib.git"),
            "ssh://example.invalid/team/lib.git"
        );
        let diagnostic = redact_git_diagnostic(
            "fatal: https://alice:topsecret@example.invalid/lib.git?password=hidden failed\n",
        );
        assert!(!diagnostic.contains("topsecret"));
        assert!(!diagnostic.contains("hidden"));
        assert!(diagnostic.contains("example.invalid/lib.git"));
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
        assert_eq!(
            shown,
            "https://example.invalid/library.git?access_token=[REDACTED]"
        );
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
}
