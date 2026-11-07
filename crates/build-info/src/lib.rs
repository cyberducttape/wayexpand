//! Build-script helper that stamps every WayExpand binary with the same
//! version and commit, so support reports from the CLI, daemon and broker
//! agree.
//!
//! Call [`emit`] from a crate's `build.rs`, then read the values with
//! `env!("WAYEXPAND_BUILD_VERSION")` and `env!("WAYEXPAND_COMMIT")`.
//! A build from the exact `v<version>` tag with a clean tree reports the
//! package version; any other git build reports `<version>-dev+<sha>`, with a
//! stable build-input digest appended when the tree is dirty.
//! `WAYEXPAND_BUILD_SHA` overrides the commit for builds without `.git`.
//! `WAYEXPAND_BUILD_RELEASE_VERSION` marks an archive as an exact release only
//! when it equals this crate's version and a source commit is supplied.

use sha2::{Digest, Sha256};
use std::{env, fmt::Write as _, fs, path::Path, process::Command};

const BUILD_INPUT_PATHS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "desktop",
    "assets",
    "systemd",
    "installer-components.toml",
    "gnome-extension",
];

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

fn ignored_build_inputs(root: &Path) -> Option<Vec<Vec<u8>>> {
    let mut arguments = vec![
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "-z",
        "--",
    ];
    arguments.extend_from_slice(BUILD_INPUT_PATHS);
    let output = git_bytes(root, &arguments)?;
    Some(
        output
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            // Cargo's target directory is generated build output, not a source
            // input, even when it is nested under one of the watched roots.
            .filter(|path| {
                !path
                    .split(|byte| *byte == b'/')
                    .any(|component| component == b"target")
            })
            .map(<[u8]>::to_vec)
            .collect(),
    )
}

fn build_inputs_are_clean(root: &Path) -> bool {
    git_worktree_is_clean(root)
        && ignored_build_inputs(root).is_some_and(|inputs| inputs.is_empty())
}

/// A stable digest of build inputs changed from HEAD, including untracked and
/// ignored source files. This distinguishes binaries built from different
/// source trees that share the same Git commit.
fn dirty_source_digest(root: &Path) -> Option<String> {
    let mut diff_arguments = vec!["diff", "--binary", "HEAD", "--"];
    diff_arguments.extend_from_slice(BUILD_INPUT_PATHS);
    let changed = git_bytes(root, &diff_arguments)?;
    let mut untracked_arguments = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    untracked_arguments.extend_from_slice(BUILD_INPUT_PATHS);
    let untracked = git_bytes(root, &untracked_arguments)?;
    let ignored = ignored_build_inputs(root)?;
    let mut hasher = Sha256::new();
    hasher.update(b"wayexpand-build-inputs-v1\0");
    hasher.update(&changed);
    for relative in untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative = std::str::from_utf8(relative).ok()?;
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path).ok()?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        if metadata.file_type().is_symlink() {
            hasher.update(fs::read_link(path).ok()?.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            hasher.update(fs::read(path).ok()?);
        }
        hasher.update([0xff]);
    }
    for relative in ignored {
        let relative_text = std::str::from_utf8(&relative).ok()?;
        let path = root.join(relative_text);
        let metadata = fs::symlink_metadata(&path).ok()?;
        hasher.update(b"ignored\0");
        hasher.update(&relative);
        hasher.update([0]);
        if metadata.file_type().is_symlink() {
            hasher.update(fs::read_link(path).ok()?.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            hasher.update(fs::read(path).ok()?);
        }
        hasher.update([0xff]);
    }
    let digest = hasher.finalize();
    let mut short = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        write!(&mut short, "{byte:02x}").ok()?;
    }
    Some(short)
}

fn git_bytes(root: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
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
        let Some(sha) = commit.filter(|sha| {
            (12..=40).contains(&sha.len()) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) else {
            return false;
        };
        if release_version_override != Some(package_version) {
            return false;
        }
        // Source archives carry an externally verified commit stamp and have
        // no repository metadata. When metadata is present, require the stamp
        // to identify this exact tagged, clean checkout as well; an override
        // must not turn arbitrary or dirty source into an exact release.
        let Some(head) = git_output(root, &["rev-parse", "HEAD"]) else {
            return true;
        };
        return head.starts_with(sha)
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
            && build_inputs_are_clean(root);
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
        && build_inputs_are_clean(root)
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
    for input in BUILD_INPUT_PATHS {
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

    let worktree_clean = build_inputs_are_clean(&root);
    let dirty_digest = (!worktree_clean)
        .then(|| dirty_source_digest(&root))
        .flatten();
    let version = match (sha.as_deref(), worktree_clean, dirty_digest.as_deref()) {
        (Some(_sha), _, _) if exact_release => package_version,
        (Some(sha), false, Some(digest)) => {
            format!("{package_version}-dev+{sha}.dirty.{digest}")
        }
        (Some(sha), false, None) => format!("{package_version}-dev+{sha}.dirty-unknown"),
        (Some(sha), true, _) => format!("{package_version}-dev+{sha}"),
        (None, _, _) => package_version,
    };
    let commit = sha.unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=WAYEXPAND_BUILD_VERSION={version}");
    println!("cargo:rustc-env=WAYEXPAND_COMMIT={commit}");
}

#[cfg(test)]
mod tests {
    use super::{
        build_inputs_are_clean, dirty_source_digest, git_worktree_is_clean, is_exact_release,
    };
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
            fs::create_dir_all(path.join("crates/cli/src")).expect("create source tree");
            fs::write(path.join("crates/cli/src/main.rs"), "fn main() {}\n")
                .expect("write source file");
            fs::write(
                path.join(".gitignore"),
                "crates/cli/src/ignored.rs\ncrates/cli/src/target/\n",
            )
            .expect("write test ignore rules");
            git_ok(
                &path,
                &["add", ".gitignore", "tracked.txt", "crates/cli/src/main.rs"],
            );
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
        let head =
            String::from_utf8(git(&repository.0, &["rev-parse", "--short=12", "HEAD"]).stdout)
                .expect("HEAD is UTF-8");
        let head = head.trim();
        assert!(is_exact_release(
            &repository.0,
            "1.3.3",
            Some(head),
            true,
            Some("1.3.3"),
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("000000000000"),
            true,
            Some("1.3.3"),
        ));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some(head),
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
            Some(head),
            true,
            Some("1.3.3"),
        ));
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

    #[test]
    fn dirty_source_digest_tracks_staged_unstaged_and_untracked_build_inputs() {
        let repository = TestRepository::new();
        let clean_digest = dirty_source_digest(&repository.0).expect("Git diff is available");

        fs::write(
            repository.0.join("crates/cli/src/main.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .expect("edit source");
        let unstaged_digest = dirty_source_digest(&repository.0).expect("hash unstaged source");
        assert_ne!(unstaged_digest, clean_digest);

        git_ok(&repository.0, &["add", "crates/cli/src/main.rs"]);
        let staged_digest = dirty_source_digest(&repository.0).expect("hash staged source");
        assert_eq!(staged_digest, unstaged_digest);

        fs::write(
            repository.0.join("crates/cli/src/new_module.rs"),
            "pub fn added() {}\n",
        )
        .expect("add untracked source");
        let untracked_digest = dirty_source_digest(&repository.0).expect("hash untracked source");
        assert_ne!(untracked_digest, staged_digest);
    }

    #[test]
    fn dirty_source_digest_tracks_root_files_embedded_into_binaries() {
        let repository = TestRepository::new();
        let clean_digest = dirty_source_digest(&repository.0).expect("Git diff is available");
        fs::write(
            repository.0.join("installer-components.toml"),
            "[components.cli]\nrequired = true\n",
        )
        .expect("write embedded installer contract");
        let changed_digest =
            dirty_source_digest(&repository.0).expect("hash embedded installer contract");
        assert_ne!(changed_digest, clean_digest);
    }

    #[test]
    fn ignored_build_sources_make_the_build_dirty_but_target_output_does_not() {
        let repository = TestRepository::new();
        let clean_digest = dirty_source_digest(&repository.0).expect("Git diff is available");
        assert!(build_inputs_are_clean(&repository.0));
        assert!(is_exact_release(
            &repository.0,
            "1.3.3",
            Some("native-commit"),
            false,
            None,
        ));

        let target_output = repository.0.join("crates/cli/src/target/generated.rs");
        fs::create_dir_all(target_output.parent().unwrap()).expect("create ignored target dir");
        fs::write(&target_output, "generated build output\n").expect("write target output");
        assert!(build_inputs_are_clean(&repository.0));
        assert_eq!(
            dirty_source_digest(&repository.0).as_deref(),
            Some(clean_digest.as_str())
        );

        let ignored_source = repository.0.join("crates/cli/src/ignored.rs");
        fs::write(&ignored_source, "pub fn altered_build_input() {}\n")
            .expect("write ignored Rust module");
        assert!(git_worktree_is_clean(&repository.0));
        assert!(!build_inputs_are_clean(&repository.0));
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some("native-commit"),
            false,
            None,
        ));
        let head =
            String::from_utf8(git(&repository.0, &["rev-parse", "--short=12", "HEAD"]).stdout)
                .expect("HEAD is UTF-8");
        assert!(!is_exact_release(
            &repository.0,
            "1.3.3",
            Some(head.trim()),
            true,
            Some("1.3.3"),
        ));
        assert_ne!(
            dirty_source_digest(&repository.0).as_deref(),
            Some(clean_digest.as_str())
        );
    }

    #[test]
    fn release_stamp_remains_supported_for_source_archives_without_git_metadata() {
        let repository = TestRepository::new();
        fs::remove_dir_all(repository.0.join(".git")).expect("remove source archive metadata");
        assert!(is_exact_release(
            &repository.0,
            "1.3.3",
            Some("abcdef123456"),
            true,
            Some("1.3.3"),
        ));
    }
}
