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
    io::Read,
    os::fd::AsRawFd,
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
                break status;
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                bail!("git {} timed out or could not be monitored", args.join(" "));
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
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
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
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git reported a non-UTF-8 path")
}

fn validate_library(config_path: &Path) -> Result<()> {
    Config::validate_library_files(config_path)
        .map_err(|error| anyhow::anyhow!("library is invalid: {}", error.safe_summary()))
}

fn remote(directory: &Path) -> Result<Option<String>> {
    // `origin` is the explicitly configured synchronization remote.  Picking
    // the first entry from `git remote` makes a repository with a backup or
    // mirror remote sync unpredictably.
    let output = git(directory, &["remote", "get-url", "origin"])?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    Ok(None)
}

/// Make the configuration directory a library repository.
pub(crate) fn init(config_path: &Path, remote_url: Option<&str>) -> Result<PathBuf> {
    let directory = config_path
        .parent()
        .context("configuration path has no directory")?
        .to_path_buf();
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
    commit_library(&directory, "Track WayExpand snippet library")?;
    Ok(directory)
}

fn commit_library(directory: &Path, message: &str) -> Result<bool> {
    let mut paths = vec![".gitignore", "expansions.toml"];
    paths.push("snippets.d");
    // `-A` is essential here: if the final snippets.d file is deleted, the
    // directory disappears and therefore cannot be selected by an existence
    // check. Git still understands the pathspec and stages the deletion.
    let mut add = vec!["add", "-A", "--"];
    for path in paths {
        let present = directory.join(path).exists();
        let tracked = !present
            && git_ok(directory, &["ls-files", "--", path]).is_ok_and(|output| !output.is_empty());
        if present || tracked {
            add.push(path);
        }
    }
    if add.len() == 3 {
        return Ok(false);
    }
    git_ok(directory, &add)?;
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
            "expansions.toml",
            "snippets.d",
        ],
    )?;
    let library_paths: Vec<String> = staged
        .split('\0')
        .filter(|path| is_library_path(path))
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
fn is_library_path(path: &str) -> bool {
    matches!(path, ".gitignore" | "expansions.toml")
        || path
            .strip_prefix("snippets.d/")
            .is_some_and(|name| !name.contains('/') && name.ends_with(".toml") && name != ".toml")
}

/// Refuse a tree unless every entry is an allowlisted library path stored as
/// an ordinary file blob (not a symlink, submodule, or other special mode).
fn verify_library_tree(directory: &Path, revision: &str) -> Result<()> {
    let listing = git_ok_raw(directory, &["ls-tree", "-r", "-z", "--full-tree", revision])?;
    let mut foreign = Vec::new();
    for entry in listing.split('\0').filter(|entry| !entry.is_empty()) {
        // `<mode> <type> <object>\t<path>`
        let (header, path) = entry
            .split_once('\t')
            .context("git ls-tree reported an unexpected entry")?;
        let mode = header.split(' ').next().unwrap_or_default();
        if !matches!(mode, "100644" | "100755") || !is_library_path(path) {
            foreign.push(path.to_owned());
        }
    }
    if foreign.is_empty() {
        return Ok(());
    }
    foreign.sort_unstable();
    bail!(
        "refusing to sync: the remote library contains entries that are not plain snippet \
         library files ({}); your local library is unchanged",
        foreign.join(", ")
    )
}

/// Refuse to push history that touches anything outside the library, such as
/// files committed by hand or by an older release. `base` is the remote tip;
/// without one, the whole history would be published and is checked.
fn verify_outgoing_paths(directory: &Path, base: Option<&str>) -> Result<()> {
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
        .filter(|path| !path.is_empty() && !is_library_path(path))
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
    validate_library(config_path)?;
    let committed = commit_library(&directory, "Sync WayExpand library")?;
    let remote = remote(&directory)?;
    let mut report = SyncReport {
        directory: directory.clone(),
        committed,
        pulled: false,
        pushed: false,
        remote: remote.clone(),
    };
    let Some(remote) = remote else {
        return Ok(report);
    };
    let before = git_ok(&directory, &["rev-parse", "HEAD"])?;
    let branch = git_ok(&directory, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let remote_has_branch = git(
        &directory,
        &["ls-remote", "--exit-code", "--heads", &remote, &branch],
    )?
    .status
    .success();
    if remote_has_branch {
        // Fetch first and inspect the remote tree before anything from it is
        // checked out: a remote must not be able to place arbitrary files,
        // symlinks, or submodules in the configuration directory.
        git_ok(&directory, &["fetch", "--quiet", &remote, &branch])?;
        verify_library_tree(&directory, "FETCH_HEAD")?;
        let rebase = git(&directory, &["rebase", "--quiet", "FETCH_HEAD"])?;
        if !rebase.status.success() {
            let _ = git(&directory, &["rebase", "--abort"]);
            restrict_library_permissions(&directory);
            bail!(
                "the remote library conflicts with local changes; your local library is unchanged. \
                 Resolve it with git in {}",
                directory.display()
            );
        }
        report.pulled = true;
        restrict_library_permissions(&directory);
        if let Err(error) = validate_library(config_path) {
            // Never leave the daemon a broken library: restore the local one.
            git_ok(&directory, &["reset", "--quiet", "--hard", &before])?;
            // The reset rewrites files with the umask; keep them loadable.
            restrict_library_permissions(&directory);
            bail!("remote changes would make the {error:#}; kept the local library");
        }
    }
    verify_outgoing_paths(&directory, remote_has_branch.then_some("FETCH_HEAD"))?;
    git_ok(
        &directory,
        &["push", "--quiet", "--set-upstream", &remote, &branch],
    )?;
    report.pushed = true;
    Ok(report)
}

/// Git writes files with the local umask; the library must not be group- or
/// world-writable to load, so pulled files are made private.
fn restrict_library_permissions(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // Never follow a symlink: chmod through one changes its target, which
    // can be any file the user owns.
    let private = |path: &Path| {
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    };
    private(&directory.join("expansions.toml"));
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "wayexpand-sync-test-{}-{suffix}",
            std::process::id()
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
        commit_library(&directory, "add snippet").expect("commit snippet");
        std::fs::remove_file(snippet).expect("delete snippet");
        std::fs::remove_dir(snippets).expect("delete empty snippets directory");
        assert!(commit_library(&directory, "remove snippet").expect("commit deletion"));
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

        restrict_library_permissions(&directory);

        let mode = std::fs::metadata(&script)
            .expect("stat script")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn library_path_allowlist_is_exact() {
        assert!(is_library_path("expansions.toml"));
        assert!(is_library_path(".gitignore"));
        assert!(is_library_path("snippets.d/work.toml"));
        assert!(!is_library_path("snippets.d/nested/work.toml"));
        assert!(!is_library_path("snippets.d/run.sh"));
        assert!(!is_library_path("portal-token"));
        assert!(!is_library_path("expansions.toml.bak"));
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
