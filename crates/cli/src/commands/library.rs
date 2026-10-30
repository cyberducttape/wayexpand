//! Offline, review-first Git sources for signed WayExpand pack catalogs.

use super::Args;
use crate::{
    args::{take_json_flag, take_option},
    commands::config,
    errors::config_error,
    sync, *,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_TREE_ENTRIES: usize = 4096;
const MAX_TREE_OUTPUT: usize = 128 * 1024;

pub(crate) fn library_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand library add <git-url> [--name NAME] [--branch BRANCH] [--json]\n       wayexpand library list [--json]\n       wayexpand library path <name>\n       wayexpand library search <name> [query] [--json]\n       wayexpand library remove <name>";
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let branch = take_option(&mut rest, "--branch")?;
    let name = take_option(&mut rest, "--name")?;
    if let Some(name) = &name {
        config::ensure_pack_id(name)?;
    }
    match rest.first().map(String::as_str) {
        Some("add") if rest.len() == 2 => {
            let remote = &rest[1];
            validate_remote(remote)?;
            let name = match name.as_deref() {
                Some(name) => name.to_owned(),
                None => library_name_from_remote(remote).ok_or_else(|| {
                    usage_error("could not derive a safe library name; provide --name NAME")
                })?,
            };
            config::ensure_pack_id(&name)?;
            let (path, revision, selected_branch, pack_count) =
                add_repository(remote, &name, branch.as_deref())?;
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": name,
                        "path": path,
                        "branch": selected_branch,
                        "revision": revision,
                        "pack_count": pack_count,
                        "installed": false,
                    })
                );
            } else {
                println!(
                    "Added {name} at {revision} ({selected_branch}); {pack_count} valid pack(s) available offline."
                );
                println!("Review with `wayexpand library search {name}`.");
                println!("No pack was installed or activated.");
            }
            Ok(())
        }
        Some("list") if rest.len() == 1 && branch.is_none() && name.is_none() => {
            list_repositories(requested_json)
        }
        Some("path") if rest.len() == 2 && branch.is_none() && name.is_none() => {
            config::ensure_pack_id(&rest[1])?;
            let path = library_root()?.join(&rest[1]);
            verify_repository_directory(&path)?;
            println!("{}", path.display());
            Ok(())
        }
        Some("search") if (2..=3).contains(&rest.len()) && branch.is_none() && name.is_none() => {
            config::ensure_pack_id(&rest[1])?;
            let path = library_root()?.join(&rest[1]);
            verify_repository_directory(&path)?;
            let policy = crate::doctor::policy::load_policy().map_err(|error| {
                config_error(format!("organization policy is invalid: {error}"))
            })?;
            if let Some(refusal) = config::pack_signers_override_refusal(&policy, false) {
                bail!("{refusal}");
            }
            let signers = Path::new(&policy.pack_signers_file);
            if policy.safe_mode && policy.require_signed_packs {
                wayexpand_core::trusted_signers_file(signers).map_err(|error| {
                    config_error(format!("pack signer trust is invalid: {error}"))
                })?;
            }
            let query = rest.get(2).map(String::as_str).unwrap_or_default();
            config::search_pack_catalog(&path, query, signers, requested_json)
        }
        Some("remove") if rest.len() == 2 && branch.is_none() && name.is_none() => {
            if requested_json {
                usage_bail!("library remove does not support --json");
            }
            remove_repository(&rest[1])
        }
        _ => usage_bail!("{USAGE}"),
    }
}

fn library_root() -> Result<PathBuf> {
    let packs = config::installed_pack_root()?;
    let data_root = packs
        .parent()
        .ok_or_else(|| config_error("local pack directory has no parent"))?;
    let root = data_root.join("libraries");
    config::prepare_private_directory(data_root)?;
    config::prepare_private_directory(&root)?;
    Ok(root)
}

fn add_repository(
    remote: &str,
    name: &str,
    branch: Option<&str>,
) -> Result<(PathBuf, String, String, usize)> {
    let root = library_root()?;
    add_repository_at(&root, remote, name, branch)
}

fn add_repository_at(
    root: &Path,
    remote: &str,
    name: &str,
    branch: Option<&str>,
) -> Result<(PathBuf, String, String, usize)> {
    config::prepare_private_directory(root)?;
    let target = root.join(name);
    if fs::symlink_metadata(&target).is_ok() {
        bail!("library {name} already exists; remove it before adding a replacement");
    }
    if let Some(branch) = branch {
        validate_branch(root, branch)?;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stage = root.join(format!(".stage-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir(&stage).map_err(|error| {
        config_error(format!(
            "could not create private library staging directory: {error}"
        ))
    })?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).map_err(|error| {
        config_error(format!(
            "could not secure library staging directory: {error}"
        ))
    })?;
    let cleanup = StagedRepository(stage.clone());

    let mut clone = vec![
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "init.templateDir=/dev/null",
        "-c",
        "core.attributesFile=/dev/null",
        "clone",
        "--quiet",
        "--depth=1",
        "--single-branch",
        "--no-tags",
        "--no-recurse-submodules",
        "--no-checkout",
        "--filter=blob:none",
    ];
    if let Some(branch) = branch {
        clone.push("--branch");
        clone.push(branch);
    }
    clone.push("--");
    clone.push(remote);
    let stage_text = stage
        .to_str()
        .ok_or_else(|| config_error("library path is not UTF-8"))?;
    clone.push(stage_text);
    let output = sync::git_library_output(root, &clone)?;
    if !output.status.success() {
        bail!(
            "could not clone library source: {}",
            sync::redact_library_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }

    validate_repository_tree(&stage)?;
    let revision = sync::git_library_ok(&stage, &["rev-parse", "HEAD"])?;
    validate_object_id(&revision)?;
    let selected_branch = sync::git_library_ok(&stage, &["branch", "--show-current"])?;
    if selected_branch.is_empty() || selected_branch == "HEAD" {
        bail!("library clone did not select a branch; use --branch to pin one");
    }
    sync::git_library_ok(
        &stage,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.attributesFile=/dev/null",
            "checkout",
            "--quiet",
            "-B",
            &selected_branch,
            "--force",
            &revision,
        ],
    )?;
    let catalog = config::scan_pack_catalog(&stage)?;
    if catalog.packs.is_empty() {
        bail!("repository contains no valid WayExpand packs");
    }
    let pack_count = catalog.packs.len();
    fs::rename(&stage, &target)
        .map_err(|error| config_error(format!("could not activate library source: {error}")))?;
    drop(cleanup);
    sync_directory(root)?;
    Ok((target, revision, selected_branch, pack_count))
}

fn validate_repository_tree(repository: &Path) -> Result<()> {
    let output =
        sync::git_library_output(repository, &["ls-tree", "-r", "-z", "--full-tree", "HEAD"])?;
    if !output.status.success() {
        bail!("could not inspect library tree before checkout");
    }
    if output.stdout.len() > MAX_TREE_OUTPUT {
        bail!("library Git tree exceeds the {MAX_TREE_OUTPUT}-byte inspection limit");
    }
    let mut entries = 0usize;
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        entries += 1;
        if entries > MAX_TREE_ENTRIES {
            bail!("library contains more than {MAX_TREE_ENTRIES} files");
        }
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| config_error("malformed Git tree entry"))?;
        let header = &record[..separator];
        let path = &record[separator + 1..];
        let mut fields = header.split(|byte| *byte == b' ');
        let mode = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        if object_type != b"blob" || !matches!(mode, b"100644" | b"100755") {
            bail!("library contains a symlink, submodule, or non-file Git entry");
        }
        if path.is_empty()
            || path[0] == b'/'
            || path
                .split(|byte| *byte == b'/')
                .any(|component| component == b".." || component == b"." || component == b".git")
            || path.iter().any(|byte| *byte < b' ' || *byte == 0x7f)
        {
            bail!("library contains a path that is unsafe to check out");
        }
    }
    Ok(())
}

fn validate_remote(remote: &str) -> Result<()> {
    if remote.is_empty()
        || remote.len() > 2048
        || remote.starts_with('-')
        || remote
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || remote.contains(['?', '#'])
    {
        bail!("library URL is empty, too long, or contains unsafe characters");
    }
    if let Some((scheme, remainder)) = remote.split_once("://") {
        if !matches!(scheme, "https" | "ssh" | "file") {
            bail!("library URL scheme must be https, ssh, or file");
        }
        let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
        if scheme == "https" && authority.contains('@') {
            bail!(
                "do not embed credentials in a library URL; use an SSH agent for private sources"
            );
        }
        if scheme == "ssh" {
            if let Some((user, _)) = authority.rsplit_once('@') {
                if user.is_empty() || user.contains(':') {
                    bail!("SSH library URLs cannot contain a password");
                }
            }
        }
        if scheme == "file" && !remainder.starts_with('/') {
            bail!("file library URLs must use an absolute path");
        }
        return Ok(());
    }
    // SCP-style SSH URLs are supported; Git's ext:: and arbitrary transport
    // helpers are deliberately excluded because they can execute programs.
    if let Some((user_host, path)) = remote.split_once(':') {
        if user_host.contains('@') && !path.is_empty() && !path.starts_with('/') {
            return Ok(());
        }
    }
    bail!("library source must be an HTTPS, SSH, or absolute file Git URL")
}

fn validate_branch(root: &Path, branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch.len() > 255
        || branch
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        bail!("branch name is invalid");
    }
    let output = sync::git_library_output(root, &["check-ref-format", "--branch", branch])?;
    if !output.status.success() {
        bail!("branch name is not a valid Git branch");
    }
    Ok(())
}

fn validate_object_id(revision: &str) -> Result<()> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("Git returned an invalid commit ID");
    }
    Ok(())
}

fn library_name_from_remote(remote: &str) -> Option<String> {
    let path = remote
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(remote)
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .strip_suffix(".git")
        .unwrap_or_else(|| {
            remote
                .trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()
                .unwrap_or_default()
        });
    let mut name = String::new();
    for character in path.chars().take(64) {
        let character = if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        {
            character.to_ascii_lowercase()
        } else {
            '-'
        };
        if character == '-' && name.ends_with('-') {
            continue;
        }
        name.push(character);
    }
    let name = name.trim_matches(['-', '.', '_']);
    (!name.is_empty() && wayexpand_core::is_valid_pack_id(name)).then(|| name.to_owned())
}

fn verify_repository_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| config_error(format!("library source is unavailable: {error}")))?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        bail!("library source must be a real, private directory owned by this user");
    }
    Ok(())
}

fn list_repositories(json_output: bool) -> Result<()> {
    let root = library_root()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|error| config_error(format!("could not read library sources: {error}")))?
    {
        let entry = entry
            .map_err(|error| config_error(format!("could not read library entry: {error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !wayexpand_core::is_valid_pack_id(&name) {
            continue;
        }
        let path = entry.path();
        verify_repository_directory(&path)?;
        let revision = sync::git_library_ok(&path, &["rev-parse", "HEAD"])?;
        validate_object_id(&revision)?;
        let branch = sync::git_library_ok(&path, &["branch", "--show-current"])?;
        entries.push(serde_json::json!({
            "name": name,
            "path": path,
            "branch": branch,
            "revision": revision,
        }));
    }
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    if json_output {
        println!("{}", serde_json::json!({"libraries": entries}));
    } else if entries.is_empty() {
        println!("No Git snippet libraries are installed.");
    } else {
        for entry in entries {
            println!(
                "{} · {} · {} · {}",
                entry["name"].as_str().unwrap_or("unknown"),
                entry["branch"].as_str().unwrap_or("detached"),
                entry["revision"].as_str().unwrap_or("unknown"),
                entry["path"].as_str().unwrap_or("unknown"),
            );
        }
    }
    Ok(())
}

fn remove_repository(name: &str) -> Result<()> {
    config::ensure_pack_id(name)?;
    let path = library_root()?.join(name);
    verify_repository_directory(&path)?;
    fs::remove_dir_all(&path)
        .map_err(|error| config_error(format!("could not remove library source: {error}")))?;
    sync_directory(path.parent().expect("library path has a parent"))?;
    println!("Removed library source {name}; installed packs are unchanged.");
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    let directory = fs::File::open(path)
        .map_err(|error| config_error(format!("could not open library directory: {error}")))?;
    directory
        .sync_all()
        .map_err(|error| config_error(format!("could not sync library directory: {error}")))
}

struct StagedRepository(PathBuf);

impl Drop for StagedRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    struct TestTree(PathBuf);

    impl TestTree {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "wayexpand-library-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let metadata = fs::symlink_metadata(&source_path).unwrap();
            if metadata.file_type().is_dir() {
                copy_tree(&source_path, &destination_path);
            } else if metadata.file_type().is_file() {
                fs::copy(source_path, destination_path).unwrap();
            } else {
                panic!("test fixture contains a non-regular file");
            }
        }
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn source_repository(root: &Path, symlink: bool) -> PathBuf {
        let repository = root.join("source");
        fs::create_dir(&repository).unwrap();
        if symlink {
            #[cfg(unix)]
            std::os::unix::fs::symlink("/etc/passwd", repository.join("packs-link")).unwrap();
        } else {
            let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs");
            copy_tree(&packs, &repository.join("packs"));
        }
        git(&repository, &["init", "--quiet", "-b", "main"]);
        git(&repository, &["config", "user.name", "WayExpand test"]);
        git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        );
        git(&repository, &["add", "--all"]);
        git(&repository, &["commit", "--quiet", "-m", "catalog"]);
        repository
    }

    #[test]
    fn add_clones_a_branch_pinned_offline_catalog_without_installing_packs() {
        let tree = TestTree::new("add");
        let repository = source_repository(&tree.0, false);
        let remote = format!("file://{}", repository.display());
        let root = tree.0.join("libraries");
        let (path, revision, branch, count) =
            add_repository_at(&root, &remote, "community", Some("main")).unwrap();

        assert!(path.join(".git").is_dir());
        assert_eq!(branch, "main");
        assert_eq!(revision.len(), 40);
        assert!(count > 0);
        assert_eq!(
            sync::git_library_ok(&path, &["rev-parse", "HEAD"]).unwrap(),
            revision
        );
        assert_eq!(
            sync::git_library_ok(&path, &["branch", "--show-current"]).unwrap(),
            "main"
        );
        assert!(!config::installed_pack_root()
            .unwrap()
            .join("linux-administration")
            .exists());
    }

    #[test]
    fn add_rejects_symlinks_before_checkout_and_cleans_staging() {
        let tree = TestTree::new("symlink");
        let repository = source_repository(&tree.0, true);
        let remote = format!("file://{}", repository.display());
        let root = tree.0.join("libraries");
        let error = add_repository_at(&root, &remote, "unsafe", Some("main"))
            .expect_err("symlink tree entries must be rejected");
        assert!(error.to_string().contains("symlink"));
        assert!(fs::read_dir(&root).unwrap().next().is_none());
    }

    #[test]
    fn remote_validation_rejects_credentials_and_exec_transports() {
        assert!(validate_remote("https://user:secret@example.invalid/catalog.git").is_err());
        assert!(validate_remote("https://example.invalid/catalog.git?token=secret").is_err());
        assert!(validate_remote("ext::sh -c evil").is_err());
        assert!(validate_remote("ssh://git@example.invalid/team/catalog.git").is_ok());
        assert!(validate_remote("git@example.invalid:team/catalog.git").is_ok());
    }
}
