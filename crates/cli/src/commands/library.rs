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
const MAX_BLOB_BYTES: u64 = 1024 * 1024;
const MAX_CHECKOUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_NETWORK_BYTES: u64 = 128 * 1024 * 1024;
const MAX_INTAKE_SECONDS: u64 = 60;

struct LibraryBudgets {
    blob_bytes: u64,
    checkout_bytes: u64,
    network_bytes: u64,
    cpu_seconds: u64,
}

impl LibraryBudgets {
    fn from_environment() -> Result<Self> {
        fn value(name: &str, default: u64, ceiling: u64) -> Result<u64> {
            let Ok(raw) = std::env::var(name) else {
                return Ok(default);
            };
            let parsed: u64 = raw
                .parse()
                .with_context(|| format!("{name} must be a positive integer"))?;
            if parsed == 0 || parsed > ceiling {
                bail!("{name} must be between 1 and {ceiling}");
            }
            Ok(parsed)
        }
        Ok(Self {
            blob_bytes: value(
                "WAYEXPAND_LIBRARY_MAX_BLOB_BYTES",
                MAX_BLOB_BYTES,
                MAX_BLOB_BYTES,
            )?,
            checkout_bytes: value(
                "WAYEXPAND_LIBRARY_MAX_CHECKOUT_BYTES",
                MAX_CHECKOUT_BYTES,
                MAX_CHECKOUT_BYTES,
            )?,
            network_bytes: value(
                "WAYEXPAND_LIBRARY_MAX_NETWORK_BYTES",
                MAX_NETWORK_BYTES,
                MAX_NETWORK_BYTES,
            )?,
            cpu_seconds: value("WAYEXPAND_LIBRARY_MAX_CPU_SECONDS", 30, MAX_INTAKE_SECONDS)?,
        })
    }
}

pub(crate) fn library_command(args: Args) -> Result<()> {
    const USAGE: &str = "usage: wayexpand library add <git-url> [--name NAME] [--branch BRANCH] [--approved-revision FULL_COMMIT] [--json]\n       wayexpand library list [--json]\n       wayexpand library path <name>\n       wayexpand library search <name> [query] [--json]\n       wayexpand library remove <name>";
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let branch = take_option(&mut rest, "--branch")?;
    let revision = take_option(&mut rest, "--approved-revision")?;
    let name = take_option(&mut rest, "--name")?;
    if let Some(revision) = &revision {
        validate_requested_revision(revision)?;
    }
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
            let revision_pinned = revision.is_some();
            let (path, revision, selected_branch, pack_count) =
                add_repository(remote, &name, branch.as_deref(), revision.as_deref())?;
            if requested_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": name,
                        "path": path,
                        "branch": selected_branch,
                        "revision": revision,
                        "pinned": revision_pinned,
                        "pack_count": pack_count,
                        "installed": false,
                    })
                );
            } else {
                println!(
                    "Added {name} at {revision} ({selected_branch}); {pack_count} valid pack(s) available offline."
                );
                if revision_pinned {
                    println!("The catalog is pinned to the approved commit {revision}.");
                }
                println!("Review with `wayexpand library search {name}`.");
                println!("No pack was installed or activated.");
            }
            Ok(())
        }
        Some("list")
            if rest.len() == 1 && branch.is_none() && revision.is_none() && name.is_none() =>
        {
            list_repositories(requested_json)
        }
        Some("path")
            if rest.len() == 2 && branch.is_none() && revision.is_none() && name.is_none() =>
        {
            config::ensure_pack_id(&rest[1])?;
            let path = library_root()?.join(&rest[1]);
            verify_repository_directory(&path)?;
            println!("{}", path.display());
            Ok(())
        }
        Some("search")
            if (2..=3).contains(&rest.len())
                && branch.is_none()
                && revision.is_none()
                && name.is_none() =>
        {
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
        Some("remove")
            if rest.len() == 2 && branch.is_none() && revision.is_none() && name.is_none() =>
        {
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
    revision: Option<&str>,
) -> Result<(PathBuf, String, String, usize)> {
    let root = library_root()?;
    add_repository_at(&root, remote, name, branch, revision)
}

fn add_repository_at(
    root: &Path,
    remote: &str,
    name: &str,
    branch: Option<&str>,
    revision: Option<&str>,
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
    let budgets = LibraryBudgets::from_environment()?;
    let git_limits = |max_file_size| sync::GitResourceLimits {
        timeout: std::time::Duration::from_secs(MAX_INTAKE_SECONDS),
        max_file_size,
        cpu_seconds: budgets.cpu_seconds,
    };

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
    let output =
        sync::git_library_output_bounded(root, &clone, None, git_limits(budgets.network_bytes))?;
    if !output.status.success() {
        bail!(
            "could not clone library source: {}",
            sync::redact_library_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }
    if String::from_utf8_lossy(&output.stderr).contains("filtering not recognized") {
        bail!("library server did not honor the filtered clone request");
    }

    if let Some(revision) = revision {
        let revision = revision.to_ascii_lowercase();
        let fetch = [
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--no-recurse-submodules",
            "--filter=blob:none",
            "origin",
            revision.as_str(),
        ];
        let output = sync::git_library_output_bounded(
            &stage,
            &fetch,
            None,
            git_limits(budgets.network_bytes),
        )?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stderr).contains("filtering not recognized")
        {
            bail!(
                "could not fetch approved catalog revision: {}",
                sync::redact_library_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
            );
        }
        let fetched = sync::git_library_ok(&stage, &["rev-parse", "FETCH_HEAD"])?;
        if !fetched.eq_ignore_ascii_case(&revision) {
            bail!("Git server returned a different commit than the approved revision");
        }
        if sync::git_library_ok(&stage, &["cat-file", "-t", "FETCH_HEAD"])? != "commit" {
            bail!("--approved-revision must identify a Git commit");
        }
        let branch = sync::git_library_ok(&stage, &["branch", "--show-current"])?;
        if branch.is_empty() || branch == "HEAD" {
            bail!("pinned catalog clone did not select a local branch");
        }
        sync::git_library_ok(
            &stage,
            &["update-ref", &format!("refs/heads/{branch}"), &revision],
        )?;
        if repository_pack_bytes(&stage)? > budgets.network_bytes {
            bail!("library download exceeds the network budget");
        }
    }

    let entries = validate_repository_tree(&stage, git_limits(budgets.network_bytes))?;
    let revision = sync::git_library_ok(&stage, &["rev-parse", "HEAD"])?;
    validate_object_id(&revision)?;
    let selected_branch = sync::git_library_ok(&stage, &["branch", "--show-current"])?;
    if selected_branch.is_empty() || selected_branch == "HEAD" {
        bail!("library clone did not select a branch; use --branch to pin one");
    }
    // Fetch only blobs below the individual limit. A server that ignores the
    // filter is rejected, and RLIMIT_FSIZE bounds any pack it writes.
    let already_downloaded = repository_pack_bytes(&stage)?;
    if already_downloaded >= budgets.network_bytes {
        bail!("library download exhausts the network budget before blob inspection");
    }
    let remaining_network = budgets.network_bytes - already_downloaded;
    let filter = format!("--filter=blob:limit={}", budgets.blob_bytes);
    let fetch = [
        "-c",
        "protocol.version=2",
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-recurse-submodules",
        "--refetch",
        filter.as_str(),
        "origin",
        revision.as_str(),
    ];
    let output =
        sync::git_library_output_bounded(&stage, &fetch, None, git_limits(remaining_network))?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stderr).contains("filtering not recognized")
    {
        bail!(
            "library bounded blob fetch failed: {}",
            sync::redact_library_diagnostic(String::from_utf8_lossy(&output.stderr).trim())
        );
    }
    if repository_pack_bytes(&stage)? > budgets.network_bytes {
        bail!("library download exceeds the network budget");
    }
    validate_fetched_blob_sizes(
        &stage,
        &entries,
        &budgets,
        git_limits(budgets.network_bytes),
    )?;
    let catalog_paths = permitted_catalog_paths(&entries)?;
    if catalog_paths.is_empty() {
        bail!("repository contains no catalog files");
    }
    let mut checkout = vec!["checkout", "--quiet", "--force", &revision, "--"];
    let pathspecs: Vec<String> = catalog_paths
        .iter()
        .map(|path| format!(":(literal){path}"))
        .collect();
    checkout.extend(pathspecs.iter().map(String::as_str));
    let output = sync::git_library_output_bounded_no_lazy_fetch(
        &stage,
        &checkout,
        None,
        git_limits(budgets.blob_bytes),
    )?;
    if !output.status.success() {
        bail!("could not check out the permitted WayExpand catalog paths");
    }
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

#[derive(Clone)]
struct TreeEntry {
    oid: String,
    path: String,
}

fn validate_repository_tree(
    repository: &Path,
    limits: sync::GitResourceLimits,
) -> Result<Vec<TreeEntry>> {
    let output = sync::git_library_output_bounded(
        repository,
        &["ls-tree", "-r", "-z", "--full-tree", "HEAD"],
        None,
        limits,
    )?;
    if !output.status.success() {
        bail!("could not inspect library tree before checkout");
    }
    if output.stdout.len() > MAX_TREE_OUTPUT {
        bail!("library Git tree exceeds the {MAX_TREE_OUTPUT}-byte inspection limit");
    }
    let mut entries = Vec::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if entries.len() >= MAX_TREE_ENTRIES {
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
        let oid = fields.next().unwrap_or_default();
        if !matches!(oid.len(), 40 | 64) || !oid.iter().all(u8::is_ascii_hexdigit) {
            bail!("library contains an invalid Git object ID");
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
        let path = std::str::from_utf8(path).context("library paths must be UTF-8")?;
        entries.push(TreeEntry {
            oid: String::from_utf8(oid.to_vec()).unwrap(),
            path: path.to_owned(),
        });
    }
    Ok(entries)
}

fn validate_fetched_blob_sizes(
    repository: &Path,
    entries: &[TreeEntry],
    budgets: &LibraryBudgets,
    limits: sync::GitResourceLimits,
) -> Result<()> {
    let mut input = Vec::with_capacity(entries.len() * 42);
    for entry in entries {
        input.extend_from_slice(entry.oid.as_bytes());
        input.push(b'\n');
    }
    let output = sync::git_library_output_bounded_no_lazy_fetch(
        repository,
        &[
            "cat-file",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ],
        Some(input),
        limits,
    )?;
    if !output.status.success() {
        bail!("could not inspect fetched library blob sizes");
    }
    let mut aggregate = 0u64;
    let records: Vec<_> = output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|r| !r.is_empty())
        .collect();
    if records.len() != entries.len() {
        bail!("library blob size inventory was incomplete");
    }
    for record in records {
        let line = std::str::from_utf8(record).context("Git returned invalid blob metadata")?;
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() != 3 || fields[1] != "blob" {
            bail!("library contains an oversized or unavailable blob");
        }
        let size: u64 = fields[2]
            .parse()
            .context("Git returned invalid blob size")?;
        if size > budgets.blob_bytes {
            bail!(
                "library contains a blob exceeding the {}-byte limit",
                budgets.blob_bytes
            );
        }
        aggregate = aggregate
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("library aggregate size overflow"))?;
        if aggregate > budgets.checkout_bytes {
            bail!(
                "library exceeds the {}-byte aggregate checkout limit",
                budgets.checkout_bytes
            );
        }
    }
    Ok(())
}

fn permitted_catalog_paths(entries: &[TreeEntry]) -> Result<Vec<String>> {
    let has_root_manifest = entries.iter().any(|e| e.path == "wayexpand-pack.toml");
    let prefix = if !has_root_manifest && entries.iter().any(|e| e.path.starts_with("packs/")) {
        "packs/"
    } else {
        ""
    };
    let mut paths = Vec::new();
    for entry in entries {
        let Some(relative) = entry.path.strip_prefix(prefix) else {
            continue;
        };
        let parts: Vec<_> = relative.split('/').collect();
        let valid = (has_root_manifest
            && matches!(relative, "wayexpand-pack.toml" | "wayexpand-pack.sig"))
            || parts.len() == 2
                && wayexpand_core::is_valid_pack_id(parts[0])
                && matches!(parts[1], "wayexpand-pack.toml" | "wayexpand-pack.sig")
            || parts.len() == 3
                && wayexpand_core::is_valid_pack_id(parts[0])
                && parts[1] == "snippets"
                && parts[2].ends_with(".toml")
                && parts[2].len() > 5;
        if valid {
            paths.push(entry.path.clone());
        }
    }
    Ok(paths)
}

fn repository_pack_bytes(repository: &Path) -> Result<u64> {
    let mut total = 0u64;
    let packs = repository.join(".git/objects/pack");
    for entry in fs::read_dir(packs).context("could not inspect Git pack storage")? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|ext| ext == "pack") {
            total = total
                .checked_add(entry.metadata()?.len())
                .ok_or_else(|| anyhow::anyhow!("Git pack size overflow"))?;
        }
    }
    Ok(total)
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

fn validate_requested_revision(revision: &str) -> Result<()> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("--approved-revision requires a full 40- or 64-character Git commit ID");
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
        git(&repository, &["config", "uploadpack.allowFilter", "true"]);
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
            add_repository_at(&root, &remote, "community", Some("main"), None).unwrap();

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
    fn add_pins_a_reviewed_full_commit_instead_of_a_moving_branch_tip() {
        let tree = TestTree::new("pinned-revision");
        let repository = source_repository(&tree.0, false);
        let approved = Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(approved.status.success());
        let approved = String::from_utf8(approved.stdout)
            .unwrap()
            .trim()
            .to_owned();
        fs::write(
            repository.join("unrelated.bin"),
            vec![0x5a; (MAX_BLOB_BYTES + 1) as usize],
        )
        .unwrap();
        git(&repository, &["add", "unrelated.bin"]);
        git(
            &repository,
            &["commit", "--quiet", "-m", "later catalog change"],
        );
        let root = tree.0.join("libraries");
        let remote = format!("file://{}", repository.display());
        let (path, revision, branch, count) =
            add_repository_at(&root, &remote, "pinned", Some("main"), Some(&approved)).unwrap();

        assert_eq!(revision, approved);
        assert_eq!(branch, "main");
        assert!(count > 0);
        assert_eq!(
            sync::git_library_ok(&path, &["rev-parse", "HEAD"]).unwrap(),
            approved
        );
        assert_ne!(
            sync::git_library_ok(&repository, &["rev-parse", "HEAD"]).unwrap(),
            revision
        );
    }

    #[test]
    fn add_rejects_symlinks_before_checkout_and_cleans_staging() {
        let tree = TestTree::new("symlink");
        let repository = source_repository(&tree.0, true);
        let remote = format!("file://{}", repository.display());
        let root = tree.0.join("libraries");
        let error = add_repository_at(&root, &remote, "unsafe", Some("main"), None)
            .expect_err("symlink tree entries must be rejected");
        assert!(error.to_string().contains("symlink"));
        assert!(fs::read_dir(&root).unwrap().next().is_none());
    }

    #[test]
    fn add_rejects_oversized_blob_before_checkout_and_cleans_staging() {
        let tree = TestTree::new("oversized-blob");
        let repository = source_repository(&tree.0, false);
        // A small tree listing and a valid catalog, with one unrelated blob
        // that exceeds the default per-blob ceiling.
        fs::write(
            repository.join("unrelated.bin"),
            vec![0x5a; (MAX_BLOB_BYTES + 1) as usize],
        )
        .unwrap();
        git(&repository, &["add", "unrelated.bin"]);
        git(&repository, &["commit", "--quiet", "-m", "oversized blob"]);
        let root = tree.0.join("libraries");
        let remote = format!("file://{}", repository.display());
        let error = add_repository_at(&root, &remote, "oversized", Some("main"), None)
            .expect_err("oversized blobs must be rejected before materialization");
        assert!(
            error.to_string().contains("oversized") || error.to_string().contains("blob"),
            "{error:#}"
        );
        assert!(!root.join("oversized").exists());
        assert!(
            fs::read_dir(&root).unwrap().next().is_none(),
            "staging data must be cleaned: {:?}",
            fs::read_dir(&root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn add_checks_out_only_the_root_pack_catalog_paths() {
        let tree = TestTree::new("root-pack");
        let repository = source_repository(&tree.0, false);
        let root_pack = repository.join("packs/linux-administration");
        let copied_pack = tree.0.join("copied-pack");
        copy_tree(&root_pack, &copied_pack);
        git(&repository, &["rm", "-qr", "packs"]);
        fs::write(repository.join("wayexpand-pack.toml"), "catalog = true\n").unwrap();
        copy_tree(&copied_pack, &repository.join("linux-administration"));
        git(&repository, &["add", "--all"]);
        git(
            &repository,
            &["commit", "--quiet", "-m", "root pack layout"],
        );
        let root = tree.0.join("libraries");
        let remote = format!("file://{}", repository.display());
        let (path, _, _, count) =
            add_repository_at(&root, &remote, "rootpack", Some("main"), None).unwrap();
        assert_eq!(count, 1);
        assert!(path.join("wayexpand-pack.toml").is_file());
        assert!(!path.join("packs").exists());
    }

    #[test]
    fn remote_validation_rejects_credentials_and_exec_transports() {
        assert!(validate_remote("https://user:secret@example.invalid/catalog.git").is_err());
        assert!(validate_remote("https://example.invalid/catalog.git?token=secret").is_err());
        assert!(validate_remote("ext::sh -c evil").is_err());
        assert!(validate_remote("ssh://git@example.invalid/team/catalog.git").is_ok());
        assert!(validate_remote("git@example.invalid:team/catalog.git").is_ok());
    }

    #[test]
    fn approved_library_revisions_must_be_full_object_ids() {
        assert!(validate_requested_revision(&"a".repeat(40)).is_ok());
        assert!(validate_requested_revision(&"A".repeat(64)).is_ok());
        assert!(validate_requested_revision("0123456789abcdef").is_err());
        assert!(validate_requested_revision(&format!("{}g", "a".repeat(39))).is_err());
    }
}
