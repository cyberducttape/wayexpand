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
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{bail, Context, Result};
use wayexpand_core::Config;

/// Track only the library; everything else in the directory stays local.
const GITIGNORE: &str = "\
# Managed by `wayexpand sync`: only the snippet library is synchronized.
*
!.gitignore
!expansions.toml
!snippets.d/
!snippets.d/*.toml
";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SyncReport {
    pub(crate) directory: PathBuf,
    pub(crate) committed: bool,
    pub(crate) pulled: bool,
    pub(crate) pushed: bool,
    pub(crate) remote: Option<String>,
}

fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .context("could not run git; is it installed?")
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

fn validate_library(config_path: &Path) -> Result<()> {
    Config::load(config_path)
        .map(|_| ())
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
    if directory.join("snippets.d").is_dir() {
        paths.push("snippets.d");
    }
    let mut add = vec!["add", "--"];
    add.extend(paths.iter().filter(|path| directory.join(path).exists()));
    git_ok(directory, &add)?;
    let staged = git(directory, &["diff", "--cached", "--quiet"])?;
    if staged.status.success() {
        return Ok(false);
    }
    git_ok(directory, &["commit", "--quiet", "-m", message]).context(
        "could not commit; set git user.name and user.email (globally or in the library repository)",
    )?;
    Ok(true)
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
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let message = format!("Sync WayExpand library from {}", host.trim());
    let committed = commit_library(&directory, message.trim_end())?;
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
        let pull = git(
            &directory,
            &["pull", "--quiet", "--rebase", &remote, &branch],
        )?;
        if !pull.status.success() {
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
    let private = |path: &Path| {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    };
    private(&directory.join("expansions.toml"));
    if let Ok(entries) = std::fs::read_dir(directory.join("snippets.d")) {
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
