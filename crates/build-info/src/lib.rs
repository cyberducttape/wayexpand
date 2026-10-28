//! Build-script helper that stamps every WayExpand binary with the same
//! version and commit, so support reports from the CLI, daemon and broker
//! agree.
//!
//! Call [`emit`] from a crate's `build.rs`, then read the values with
//! `env!("WAYEXPAND_BUILD_VERSION")` and `env!("WAYEXPAND_COMMIT")`.
//! A build from the exact `v<version>` tag with a clean tree reports the
//! package version; any other git build reports `<version>-dev+<sha>`.
//! `WAYEXPAND_BUILD_SHA` overrides the commit for builds without `.git`.
//! `WAYEXPAND_BUILD_RELEASE_VERSION` marks an archive as an exact release only
//! when it equals this crate's version and a source commit is supplied.

use std::{env, path::Path, process::Command};

fn git_output(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn git_worktree_is_clean(root: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout.is_empty())
}

fn is_exact_release(
    root: &Path,
    package_version: &str,
    commit: Option<&str>,
    commit_overridden: bool,
    release_version_override: Option<&str>,
) -> bool {
    if commit.is_none() {
        return false;
    }
    if commit_overridden {
        return release_version_override == Some(package_version)
            && commit.is_some_and(|sha| {
                (12..=40).contains(&sha.len()) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
    }
    release_version_override.is_none()
        && git_output(
            root,
            &[
                "describe",
                "--tags",
                "--exact-match",
                "--match",
                &format!("v{package_version}"),
            ],
        )
        .is_some()
        && git_worktree_is_clean(root)
}

/// Emit `WAYEXPAND_BUILD_VERSION` and `WAYEXPAND_COMMIT` for the calling
/// crate, plus the rerun triggers that keep them current across commits.
pub fn emit() {
    println!("cargo:rerun-if-env-changed=WAYEXPAND_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=WAYEXPAND_BUILD_RELEASE_VERSION");

    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    // Source edits do not change Git's index. Watching the workspace source
    // tree ensures this helper runs again after tracked edits or source files
    // are added, so the release cleanliness check cannot reuse an old result.
    for input in ["Cargo.toml", "Cargo.lock", "crates"] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    // `.git/HEAD` only names the current branch, so it does not change on a
    // commit; the branch ref (or `packed-refs`) does. Watching only HEAD left
    // `--version` reporting the previous commit after every commit. Paths are
    // resolved through git so linked worktrees work too.
    let mut watched = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    watched.extend(git_output(&root, &["symbolic-ref", "-q", "HEAD"]));
    for name in watched {
        // `join` keeps an absolute path as is. A missing path would make
        // cargo rerun the build script on every build.
        if let Some(path) = git_output(&root, &["rev-parse", "--git-path", &name])
            .map(|path| root.join(path))
            .filter(|path| path.exists())
        {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let package_version = env::var("CARGO_PKG_VERSION").unwrap();
    let build_sha = env::var("WAYEXPAND_BUILD_SHA")
        .ok()
        .filter(|value| !value.is_empty());
    let release_version = env::var("WAYEXPAND_BUILD_RELEASE_VERSION")
        .ok()
        .filter(|value| !value.is_empty());
    let sha = build_sha
        .clone()
        .or_else(|| git_output(&root, &["rev-parse", "--short=12", "HEAD"]));
    let exact_release = is_exact_release(
        &root,
        &package_version,
        sha.as_deref(),
        build_sha.is_some(),
        release_version.as_deref(),
    );

    let version = match sha.as_deref() {
        Some(_sha) if exact_release => package_version,
        Some(sha) => format!("{package_version}-dev+{sha}"),
        None => package_version,
    };
    let commit = sha.unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=WAYEXPAND_BUILD_VERSION={version}");
    println!("cargo:rustc-env=WAYEXPAND_COMMIT={commit}");
}

#[cfg(test)]
mod tests {
    use super::{git_worktree_is_clean, is_exact_release};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestRepository(PathBuf);

    impl TestRepository {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "wayexpand-build-info-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temporary git repository");
            git_ok(&path, &["init", "--quiet"]);
            git_ok(&path, &["config", "user.name", "WayExpand Test"]);
            git_ok(
                &path,
                &["config", "user.email", "wayexpand-test@example.invalid"],
            );
            fs::write(path.join("tracked.txt"), "initial\n").expect("write tracked file");
            git_ok(&path, &["add", "tracked.txt"]);
            git_ok(&path, &["commit", "--quiet", "-m", "initial"]);
            git_ok(&path, &["tag", "v1.3.3"]);
            Self(path)
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn git(root: &Path, arguments: &[&str]) -> Output {
        Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .expect("run git")
    }

    fn git_ok(root: &Path, arguments: &[&str]) {
        let output = git(root, arguments);
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn release_cleanliness_rejects_staged_and_untracked_changes() {
        let repository = TestRepository::new();
        assert!(git_worktree_is_clean(&repository.0));
        assert!(is_exact_release(
            &repository.0,
            "1.3.3",
            Some("native-commit"),
            false,
            None,
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("override-commit"),
            true,
            None,
        ));
        assert!(is_exact_release(
            &repository.0,
            "1.3.3",
            Some("abcdef123456"),
            true,
            Some("1.3.3"),
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("abcdef123456"),
            true,
            Some("1.3.4"),
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            None,
            true,
            Some("1.3.3"),
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("unknown"),
            true,
            Some("1.3.3"),
        ));

        fs::write(repository.0.join("tracked.txt"), "unstaged edit\n").expect("edit tracked file");
        assert!(!git_worktree_is_clean(&repository.0));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("native-commit"),
            false,
            None,
        ));

        fs::write(repository.0.join("tracked.txt"), "staged edit\n").expect("edit tracked file");
        git_ok(&repository.0, &["add", "tracked.txt"]);
        assert!(!git_worktree_is_clean(&repository.0));

        git_ok(&repository.0, &["reset", "--hard", "--quiet", "HEAD"]);
        fs::write(repository.0.join("untracked-secret.txt"), "untracked\n")
            .expect("write untracked file");
        assert!(!git_worktree_is_clean(&repository.0));
    }
}
