//! Fleet layer and pack discovery.

use super::{require_root_owned, ConfigError, FleetError, Layer, OrganizationPolicy, Provenance};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(super) fn standard_layer_dirs() -> Result<Vec<(PathBuf, String)>, FleetError> {
    let mut sources = Vec::new();
    for layer in [Layer::Organization, Layer::User, Layer::Pack] {
        let Some(dir) = layer.default_dir() else {
            continue;
        };
        if !dir.exists() {
            continue;
        }
        sources.extend(discover_layer_dirs(&dir, layer)?);
    }
    Ok(sources)
}

pub(super) fn discover_layer_dirs(
    dir: &Path,
    layer: Layer,
) -> Result<Vec<(PathBuf, String)>, FleetError> {
    if layer != Layer::Pack {
        return Ok(vec![(dir.to_path_buf(), layer.name().to_string())]);
    }
    let mut dirs = vec![(dir.to_path_buf(), "pack:root".to_string())];
    dirs.extend(
        discover_pack_dirs(dir)?
            .into_iter()
            .map(|(pack_dir, name)| (pack_dir, format!("pack:{name}"))),
    );
    Ok(dirs)
}

pub(super) fn discover_layer_files(
    dir: &Path,
    is_organization: bool,
) -> Result<Vec<PathBuf>, FleetError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    // A manifest pack (`wayexpand pack` format) keeps its snippets in
    // `snippets/`; its manifest and signature are not snippet files.
    if !is_organization && dir.join("wayexpand-pack.toml").is_file() {
        return discover_layer_files(&dir.join("snippets"), false);
    }
    if is_organization {
        let metadata = fs::symlink_metadata(dir).map_err(|source| {
            FleetError::Config(ConfigError::Read {
                path: dir.display().to_string(),
                source,
            })
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
            return Err(FleetError::InvalidOrganizationDirectory {
                path: dir.display().to_string(),
            });
        }
        require_root_owned(dir, &metadata)?;
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| {
        FleetError::Config(ConfigError::Read {
            path: dir.display().to_string(),
            source,
        })
    })? {
        let entry = entry.map_err(|source| {
            FleetError::Config(ConfigError::Read {
                path: dir.display().to_string(),
                source,
            })
        })?;
        if entry.file_name().to_string_lossy().ends_with(".toml") {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

pub(super) fn discover_pack_dirs(dir: &Path) -> Result<Vec<(PathBuf, String)>, FleetError> {
    let mut pack_dirs = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| {
        FleetError::Config(ConfigError::Read {
            path: dir.display().to_string(),
            source,
        })
    })? {
        let entry = entry.map_err(|source| {
            FleetError::Config(ConfigError::Read {
                path: dir.display().to_string(),
                source,
            })
        })?;
        if entry
            .file_type()
            .map_err(|source| {
                FleetError::Config(ConfigError::Read {
                    path: entry.path().display().to_string(),
                    source,
                })
            })?
            .is_dir()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            pack_dirs.push((entry.path(), name));
        }
    }
    pack_dirs.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(pack_dirs)
}

pub(super) fn pack_name(provenance: &Provenance) -> &str {
    provenance
        .layer
        .strip_prefix("pack:")
        .or_else(|| provenance.file.split('/').next())
        .unwrap_or(provenance.file.as_str())
        .trim_end_matches(".toml")
}

pub(super) fn settings_source_allowed(
    provenance: &Provenance,
    policy: &OrganizationPolicy,
) -> bool {
    if !policy.safe_mode || policy.allowed_packs.is_empty() {
        return true;
    }
    provenance.layer == "organization"
        || provenance.layer == "user"
        || !provenance.layer.starts_with("pack:")
        || policy.pack_allowed(pack_name(provenance))
}
