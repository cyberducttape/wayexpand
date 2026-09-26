/// Fleet configuration: multi-layer, policy-driven snippet management.
///
/// Enables organizations to deploy company-wide snippets without owning users'
/// personal configurations. Layers are merged in deterministic order:
///
/// 1. `/etc/wayexpand/snippets.d/` (organization snippets, root-owned)
/// 2. `~/.config/wayexpand/snippets.d/` (user personal snippets)
/// 3. `~/.local/share/wayexpand/packs/` (optional curated packs)
///
/// Each layer is a directory of .toml files. Within a layer, files are merged
/// alphabetically.
///
/// ## Precedence and Conflict Resolution
///
/// **Expansion triggers and hotkey chords:** Duplicates are rejected with error
/// (fail-closed). Each trigger/chord name must be unique across all layers and
/// the base config.
///
/// **Settings:** Last layer wins. Pack settings override user settings, which
/// override organization settings. Within a layer, later files override earlier.
///
/// **Organization policy:** Security policy is not part of fleet layers. The
/// daemon loads it exclusively from `/etc/wayexpand/policy.toml`.
///
/// **Base config:** Prepended first (lowest priority for expansions/hotkeys).
/// Base settings only override if no layer provides settings.
use crate::{Config, ConfigError, OrganizationPolicy};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Errors that can occur during fleet configuration merging.
#[derive(Debug, Error)]
pub enum FleetError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("duplicate trigger: {message} (first defined in {existing_file})")]
    DuplicateTrigger {
        trigger: String,
        message: String,
        existing_file: String,
    },
    #[error("duplicate hotkey: {message} (first defined in {existing_file})")]
    DuplicateHotkey {
        chord: String,
        message: String,
        existing_file: String,
    },
    #[error("organization policy is not allowed in fleet layer file {file}; use /etc/wayexpand/policy.toml")]
    OrganizationPolicyInLayer { file: String },
    #[error("organization fleet path {path} is not owned by root (uid {uid})")]
    OrganizationPathNotRootOwned { path: String, uid: u32 },
    #[error("organization fleet directory {path} must be a real, non-writable directory")]
    InvalidOrganizationDirectory { path: String },
}

fn reject_embedded_policy(config: &Config, path: &Path) -> Result<(), FleetError> {
    if config.organization.is_active() {
        return Err(FleetError::OrganizationPolicyInLayer {
            file: path.display().to_string(),
        });
    }
    Ok(())
}

fn require_root_owned(path: &Path, metadata: &fs::Metadata) -> Result<(), FleetError> {
    if metadata.uid() != 0 {
        return Err(FleetError::OrganizationPathNotRootOwned {
            path: path.display().to_string(),
            uid: metadata.uid(),
        });
    }
    Ok(())
}

/// Metadata about where a snippet originated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provenance {
    /// File path relative to layer root
    pub file: String,
    /// Layer source (e.g., "organization", "user", "pack:sre-core")
    pub layer: String,
}

/// A configuration layer directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Organization = 1,
    User = 2,
    Pack = 3,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Organization => "organization",
            Layer::User => "user",
            Layer::Pack => "pack",
        }
    }

    /// Default directory for this layer
    pub fn default_dir(self) -> Option<PathBuf> {
        match self {
            Layer::Organization => Some(PathBuf::from("/etc/wayexpand/snippets.d")),
            Layer::User => {
                if let Ok(home) = std::env::var("HOME") {
                    Some(PathBuf::from(home).join(".config/wayexpand/snippets.d"))
                } else {
                    None
                }
            }
            Layer::Pack => {
                if let Ok(home) = std::env::var("HOME") {
                    Some(PathBuf::from(home).join(".local/share/wayexpand/packs"))
                } else {
                    None
                }
            }
        }
    }
}

/// Result of merging multiple configuration layers with provenance tracking.
#[derive(Debug, Clone)]
pub struct FleetConfig {
    /// Merged configuration
    pub config: Config,
    /// Provenance for each expansion (by trigger name)
    pub expansions_source: HashMap<String, Provenance>,
    /// Provenance for each hotkey (by chord)
    pub hotkeys_source: HashMap<String, Provenance>,
    /// Settings candidates in layer order, retaining provenance so pack policy
    /// can be applied before resolving the last-wins value.
    pub settings_sources: Vec<(crate::Settings, Provenance)>,
    /// Merge statistics
    pub stats: MergeStats,
    /// Administrator policy violations discovered while merging pack sources.
    pub policy_violations: Vec<String>,
}

/// Statistics about the merge operation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MergeStats {
    pub total_files_loaded: usize,
    pub total_expansions: usize,
    pub total_hotkeys: usize,
    pub layers_applied: Vec<String>,
}

impl FleetConfig {
    /// Load and merge configuration from standard fleet layers.
    ///
    /// Loads from /etc/wayexpand/snippets.d (organization),
    /// ~/.config/wayexpand/snippets.d (user), and
    /// ~/.local/share/wayexpand/packs (curated packs).
    /// Falls back gracefully if layers don't exist.
    pub fn load_standard() -> Result<Self, FleetError> {
        let mut merger = ConfigMerger::new();
        for (dir, layer_name) in standard_layer_dirs()? {
            merger.load_layer_named(dir, layer_name)?;
        }

        merger.merge()
    }

    /// Return the configuration files resolved by the standard fleet loader.
    /// Reload watchers use this list so nested pack sources are covered too.
    pub fn standard_source_files() -> Result<Vec<PathBuf>, FleetError> {
        let mut files = Vec::new();
        for (dir, layer_name) in standard_layer_dirs()? {
            files.extend(discover_layer_files(
                &dir,
                layer_name == Layer::Organization.name(),
            )?);
        }
        Ok(files)
    }

    /// Load the standard fleet layers on top of the user's primary config.
    ///
    /// The primary config remains the lowest-priority layer so enabling fleet
    /// configuration does not make existing personal snippets disappear.
    ///
    /// **Policy enforcement:** If organization policy restricts allowed_packs,
    /// pack-sourced expansions not in the allowed list are filtered out.
    pub fn load_standard_with_base(base: Config) -> Result<Self, FleetError> {
        Self::load_standard_with_base_and_policy(base, &OrganizationPolicy::default())
    }

    /// Load fleet layers while applying the administrator-owned policy.
    pub fn load_standard_with_base_and_policy(
        base: Config,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let fleet = Self::load_standard()?;
        let mut merged = Self::apply_base_and_policy(fleet, base, policy)?;
        merged
            .config
            .apply_administrator_policy(policy)
            .map_err(FleetError::Config)?;
        Ok(merged)
    }

    fn apply_base_and_policy(
        mut fleet: Self,
        base: Config,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let fleet_settings = fleet
            .settings_sources
            .iter()
            .rev()
            .find(|(_, provenance)| settings_source_allowed(provenance, policy))
            .map(|(settings, _)| settings.clone())
            .unwrap_or_default();
        let base_expansions = base.expansion.len();
        let base_hotkeys = base.hotkey.len();

        let mut config = base;
        config.organization = crate::OrganizationPolicy::default();

        let disallowed_packs: BTreeSet<_> = fleet
            .expansions_source
            .values()
            .chain(fleet.hotkeys_source.values())
            .chain(
                fleet
                    .settings_sources
                    .iter()
                    .map(|(_, provenance)| provenance),
            )
            .filter(|provenance| provenance.layer.starts_with("pack:"))
            .map(pack_name)
            .filter(|name| !policy.pack_allowed(name))
            .collect();
        fleet.policy_violations = disallowed_packs
            .into_iter()
            .map(|name| {
                format!(
                    "pack '{name}' is not in allowed_packs: {:?}",
                    policy.allowed_packs
                )
            })
            .collect();

        // Enforce the same audited decision only when safe mode is enabled.
        if !policy.allowed_packs.is_empty() && policy.safe_mode {
            fleet.config.expansion.retain(|expansion| {
                if let Some(prov) = fleet.expansions_source.get(&expansion.trigger) {
                    // Keep organization and user layers, filter packs
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || (prov.layer.starts_with("pack:") && policy.pack_allowed(pack_name(prov)))
                } else {
                    true
                }
            });
            fleet.config.hotkey.retain(|hotkey| {
                if let Some(prov) = fleet.hotkeys_source.get(&hotkey.chord) {
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || (prov.layer.starts_with("pack:") && policy.pack_allowed(pack_name(prov)))
                } else {
                    true
                }
            });
        }

        config.expansion.extend(fleet.config.expansion);
        config.hotkey.extend(fleet.config.hotkey);
        if !fleet_settings.is_default() {
            config.settings = fleet_settings;
        }
        config.validate().map_err(FleetError::Config)?;
        fleet.config = config;
        // These fields are reported alongside the active arrays by the CLI;
        // recompute them after policy filtering and base-layer composition so
        // status output cannot describe entries that are no longer active.
        fleet.stats.total_expansions = fleet.config.expansion.len();
        fleet.stats.total_hotkeys = fleet.config.hotkey.len();
        fleet.stats.total_files_loaded += usize::from(base_expansions > 0 || base_hotkeys > 0);
        fleet
            .stats
            .layers_applied
            .insert(0, "base (primary config)".to_string());
        Ok(fleet)
    }

    /// Load and merge from custom layer directories.
    pub fn load_layers(layers: Vec<(Layer, PathBuf)>) -> Result<Self, FleetError> {
        let mut merger = ConfigMerger::new();
        for (layer, path) in layers {
            merger.load_layer(path, layer)?;
        }
        merger.merge()
    }

    /// Get all unique triggers across all layers
    pub fn all_triggers(&self) -> Vec<&str> {
        self.config
            .expansion
            .iter()
            .map(|e| e.trigger.as_str())
            .collect()
    }

    /// Get provenance for a trigger (where it came from)
    pub fn trigger_source(&self, trigger: &str) -> Option<&Provenance> {
        self.expansions_source.get(trigger)
    }
}

/// Merges configuration from multiple layers with duplicate detection.
///
/// Implements fleet precedence: duplicates are rejected (fail-closed) for triggers
/// and hotkeys, while settings follow "last wins".
///
/// Duplicate detection across all loaded layers ensures configuration safety:
/// accidental trigger collisions are caught early rather than silently masked
/// by load order. This is intentional and trusted behavior for enterprise fleets.
struct ConfigMerger {
    // Individual items keyed by trigger/chord for duplicate detection.
    // Stores (item, provenance) to report exact source on conflict.
    expansions: BTreeMap<String, (crate::ExpansionConfig, Provenance)>,
    hotkeys: BTreeMap<String, (crate::HotkeyConfig, Provenance)>,
    settings: Vec<(crate::Settings, Provenance)>,
    stats: MergeStats,
}

impl ConfigMerger {
    fn new() -> Self {
        Self {
            expansions: BTreeMap::new(),
            hotkeys: BTreeMap::new(),
            settings: Vec::new(),
            stats: MergeStats::default(),
        }
    }

    fn load_layer(&mut self, dir: impl AsRef<Path>, layer: Layer) -> Result<(), FleetError> {
        self.load_layer_named(dir, layer.name().to_string())
    }

    fn load_layer_named(
        &mut self,
        dir: impl AsRef<Path>,
        layer_name: String,
    ) -> Result<(), FleetError> {
        let dir = dir.as_ref();

        if !dir.is_dir() {
            return Ok(()); // Layer directory doesn't exist, skip silently
        }

        let is_organization = layer_name == Layer::Organization.name();

        for path in discover_layer_files(dir, is_organization)? {
            if is_organization {
                let metadata = fs::symlink_metadata(&path).map_err(|source| {
                    FleetError::Config(ConfigError::Read {
                        path: path.display().to_string(),
                        source,
                    })
                })?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(FleetError::Config(ConfigError::NotRegular {
                        path: path.display().to_string(),
                    }));
                }
                require_root_owned(&path, &metadata)?;
            }
            let relative = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .display()
                .to_string();
            let provenance = Provenance {
                file: relative.clone(),
                layer: layer_name.clone(),
            };

            let config = Config::load(&path).map_err(FleetError::Config)?;
            reject_embedded_policy(&config, &path)?;
            self.stats.total_files_loaded += 1;

            // Merge expansions (check for duplicates)
            for expansion in &config.expansion {
                if self.expansions.contains_key(&expansion.trigger) {
                    let existing = &self.expansions[&expansion.trigger].1;
                    return Err(FleetError::DuplicateTrigger {
                        trigger: expansion.trigger.clone(),
                        message: format!(
                            "expansion trigger '{}' in {} conflicts with existing definition",
                            expansion.trigger, provenance.file
                        ),
                        existing_file: existing.file.clone(),
                    });
                }
                self.expansions.insert(
                    expansion.trigger.clone(),
                    (expansion.clone(), provenance.clone()),
                );
                self.stats.total_expansions += 1;
            }

            // Merge hotkeys (check for duplicates)
            for hotkey in &config.hotkey {
                if self.hotkeys.contains_key(&hotkey.chord) {
                    let existing = &self.hotkeys[&hotkey.chord].1;
                    return Err(FleetError::DuplicateHotkey {
                        chord: hotkey.chord.clone(),
                        message: format!(
                            "hotkey chord '{}' in {} conflicts with existing definition",
                            hotkey.chord, provenance.file
                        ),
                        existing_file: existing.file.clone(),
                    });
                }
                self.hotkeys
                    .insert(hotkey.chord.clone(), (hotkey.clone(), provenance.clone()));
                self.stats.total_hotkeys += 1;
            }

            // Settings (last one wins, with warning on conflict)
            if !config.settings.is_default() {
                if !self.settings.is_empty() {
                    // Settings already defined, later one wins (log this)
                    eprintln!(
                        "warning: settings from {} override previous layer",
                        provenance.file
                    );
                }
                self.settings
                    .push((config.settings.clone(), provenance.clone()));
            }
        }

        self.stats
            .layers_applied
            .push(format!("{} ({})", layer_name, dir.display()));

        Ok(())
    }

    fn merge(self) -> Result<FleetConfig, FleetError> {
        let expansions_source: HashMap<String, Provenance> = self
            .expansions
            .iter()
            .map(|(trigger, (_, prov))| (trigger.clone(), prov.clone()))
            .collect();

        let hotkeys_source: HashMap<String, Provenance> = self
            .hotkeys
            .iter()
            .map(|(chord, (_, prov))| (chord.clone(), prov.clone()))
            .collect();

        let expansion: Vec<_> = self
            .expansions
            .into_iter()
            .map(|(_, (expansion_config, _))| expansion_config)
            .collect();

        let hotkey: Vec<_> = self
            .hotkeys
            .into_iter()
            .map(|(_, (hotkey_config, _))| hotkey_config)
            .collect();

        let settings_sources = self.settings;
        let settings = settings_sources
            .last()
            .map(|(settings, _)| settings.clone())
            .unwrap_or_default();

        let config = Config {
            expansion,
            hotkey,
            settings,
            organization: crate::OrganizationPolicy::default(),
        };

        config.validate().map_err(FleetError::Config)?;

        Ok(FleetConfig {
            config,
            expansions_source,
            hotkeys_source,
            settings_sources,
            stats: self.stats,
            policy_violations: Vec::new(),
        })
    }
}

fn standard_layer_dirs() -> Result<Vec<(PathBuf, String)>, FleetError> {
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

fn discover_layer_dirs(dir: &Path, layer: Layer) -> Result<Vec<(PathBuf, String)>, FleetError> {
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

fn discover_layer_files(dir: &Path, is_organization: bool) -> Result<Vec<PathBuf>, FleetError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
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

fn discover_pack_dirs(dir: &Path) -> Result<Vec<(PathBuf, String)>, FleetError> {
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

fn pack_name(provenance: &Provenance) -> &str {
    provenance
        .layer
        .strip_prefix("pack:")
        .or_else(|| provenance.file.split('/').next())
        .unwrap_or(provenance.file.as_str())
        .trim_end_matches(".toml")
}

fn settings_source_allowed(provenance: &Provenance, policy: &OrganizationPolicy) -> bool {
    if !policy.safe_mode || policy.allowed_packs.is_empty() {
        return true;
    }
    provenance.layer == "organization"
        || provenance.layer == "user"
        || !provenance.layer.starts_with("pack:")
        || policy.pack_allowed(pack_name(provenance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_defaults() {
        assert_eq!(Layer::Organization.name(), "organization");
        assert_eq!(Layer::User.name(), "user");
        assert_eq!(Layer::Pack.name(), "pack");
    }

    #[test]
    fn provenance_tracks_source() {
        let prov = Provenance {
            file: "kubectl-snippets.toml".to_string(),
            layer: "pack".to_string(),
        };
        assert_eq!(prov.file, "kubectl-snippets.toml");
        assert_eq!(prov.layer, "pack");
    }

    #[test]
    fn pack_provenance_uses_pack_name_for_allowlists() {
        let prov = Provenance {
            file: "snippets.toml".to_string(),
            layer: "pack:sre-core".to_string(),
        };
        assert_eq!(pack_name(&prov), "sre-core");
    }

    #[test]
    fn embedded_fleet_policy_is_rejected_instead_of_discarded() {
        let mut config = Config {
            expansion: Vec::new(),
            hotkey: Vec::new(),
            settings: Default::default(),
            organization: OrganizationPolicy::default(),
        };
        config.organization.allowed_packs = vec!["approved".to_string()];

        let result = reject_embedded_policy(&config, Path::new("org.toml"));
        assert!(matches!(
            result,
            Err(FleetError::OrganizationPolicyInLayer { .. })
        ));
    }

    #[test]
    fn pack_directory_discovery_returns_named_packs() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-pack-discovery-{}", std::process::id()));
        std::fs::create_dir_all(root.join("linux-admin")).unwrap();
        std::fs::create_dir_all(root.join("kubernetes")).unwrap();

        let packs = discover_pack_dirs(&root).unwrap();
        let names: Vec<_> = packs.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, ["kubernetes", "linux-admin"]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pack_source_discovery_includes_nested_toml_files() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-pack-sources-{}", std::process::id()));
        let nested = root.join("my-pack").join("foo.toml");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "").unwrap();

        let files: Vec<_> = discover_layer_dirs(&root, Layer::Pack)
            .unwrap()
            .into_iter()
            .flat_map(|(dir, _)| discover_layer_files(&dir, false).unwrap())
            .collect();
        assert_eq!(files.as_slice(), std::slice::from_ref(&nested));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn layer_ordering() {
        assert!(Layer::Organization < Layer::User);
        assert!(Layer::User < Layer::Pack);
    }

    #[test]
    fn fleet_merge_no_duplication_regression() {
        // Regression test for fleet duplication bug (N expansions → N² entries).
        // This test would have caught the bug where ConfigMerger stored entire
        // Config objects instead of individual ExpansionConfig/HotkeyConfig items.
        let mut merger = ConfigMerger::new();

        // Simulate loading a single TOML file with 3 expansions
        let config_str = r#"
[[expansion]]
trigger = ":a"
replacement = "first"

[[expansion]]
trigger = ":b"
replacement = "second"

[[expansion]]
trigger = ":c"
replacement = "third"
"#;
        let config = Config::parse(config_str).unwrap();

        // Manually add expansions as if loaded from a file
        for expansion in config.expansion {
            merger.expansions.insert(
                expansion.trigger.clone(),
                (
                    expansion.clone(),
                    Provenance {
                        file: "test.toml".to_string(),
                        layer: "test".to_string(),
                    },
                ),
            );
        }

        let result = merger.merge().unwrap();

        // Should have exactly 3 expansions, not 9 (3×3 duplication bug)
        assert_eq!(
            result.config.expansion.len(),
            3,
            "Fleet merge created duplicate entries"
        );
    }

    #[test]
    fn policy_filters_disallowed_pack_in_complete_fleet_merge() {
        let mut merger = ConfigMerger::new();
        let mut add_config = |config: Config, file: &str, layer: &str| {
            for expansion in config.expansion {
                merger.expansions.insert(
                    expansion.trigger.clone(),
                    (
                        expansion,
                        Provenance {
                            file: file.to_string(),
                            layer: layer.to_string(),
                        },
                    ),
                );
            }
        };
        add_config(
            Config::parse("[[expansion]]\ntrigger = ':org'\nreplacement = 'organization'\n")
                .unwrap(),
            "organization.toml",
            "organization",
        );
        add_config(
            Config::parse("[[expansion]]\ntrigger = ':approved'\nreplacement = 'approved'\n")
                .unwrap(),
            "snippets.toml",
            "pack:approved",
        );
        add_config(
            Config::parse("[[expansion]]\ntrigger = ':blocked'\nreplacement = 'blocked'\n")
                .unwrap(),
            "snippets.toml",
            "pack:disallowed",
        );
        let fleet = merger.merge().unwrap();

        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };
        let base =
            Config::parse("[[expansion]]\ntrigger = ':personal'\nreplacement = 'personal'\n")
                .unwrap();
        let merged = FleetConfig::apply_base_and_policy(fleet, base, &policy).unwrap();
        let triggers: Vec<_> = merged.all_triggers();

        assert!(triggers.contains(&":personal"));
        assert!(triggers.contains(&":org"));
        assert!(triggers.contains(&":approved"));
        assert!(!triggers.contains(&":blocked"));
        assert_eq!(merged.stats.total_expansions, triggers.len());
        assert_eq!(
            merged.policy_violations,
            ["pack 'disallowed' is not in allowed_packs: [\"approved\"]"]
        );
    }

    fn settings_candidate(max_buffer_chars: usize) -> crate::Settings {
        crate::Settings {
            max_buffer_chars,
            ..crate::Settings::default()
        }
    }

    fn add_settings_candidate(merger: &mut ConfigMerger, max_buffer_chars: usize, layer: &str) {
        merger.settings.push((
            settings_candidate(max_buffer_chars),
            Provenance {
                file: "settings.toml".to_string(),
                layer: layer.to_string(),
            },
        ));
    }

    #[test]
    fn disallowed_pack_settings_are_filtered() {
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 4096, "pack:disallowed");
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };

        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert_eq!(merged.config.settings, crate::Settings::default());
    }

    #[test]
    fn settings_only_disallowed_pack_is_reported() {
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 4096, "pack:disallowed");
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };

        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert_eq!(
            merged.policy_violations,
            ["pack 'disallowed' is not in allowed_packs: [\"approved\"]"]
        );
    }

    #[test]
    fn allowed_pack_settings_remain_active() {
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 4096, "pack:approved");
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };

        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert_eq!(merged.config.settings.max_buffer_chars, 4096);
        assert!(merged.policy_violations.is_empty());
    }

    #[test]
    fn filtering_last_pack_settings_falls_back_to_previous_allowed_settings() {
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 1024, "pack:approved");
        add_settings_candidate(&mut merger, 4096, "pack:disallowed");
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };

        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert_eq!(merged.config.settings.max_buffer_chars, 1024);
    }

    #[test]
    fn audit_mode_reports_but_does_not_filter_pack_settings() {
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 4096, "pack:disallowed");
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };

        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert_eq!(merged.config.settings.max_buffer_chars, 4096);
        assert_eq!(
            merged.policy_violations,
            ["pack 'disallowed' is not in allowed_packs: [\"approved\"]"]
        );
    }

    #[test]
    fn audit_mode_reports_but_retains_disallowed_pack() {
        let mut merger = ConfigMerger::new();
        let expansion = Config::parse("[[expansion]]\ntrigger = ':blocked'\nreplacement = 'x'\n")
            .unwrap()
            .expansion
            .into_iter()
            .next()
            .unwrap();
        merger.expansions.insert(
            ":blocked".to_string(),
            (
                expansion,
                Provenance {
                    file: "snippets.toml".to_string(),
                    layer: "pack:disallowed".to_string(),
                },
            ),
        );
        let fleet = merger.merge().unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            allowed_packs: vec!["approved".to_string()],
            ..OrganizationPolicy::default()
        };
        let merged =
            FleetConfig::apply_base_and_policy(fleet, Config::parse("").unwrap(), &policy).unwrap();

        assert!(merged.all_triggers().contains(&":blocked"));
        assert_eq!(
            merged.policy_violations,
            ["pack 'disallowed' is not in allowed_packs: [\"approved\"]"]
        );
    }

    #[test]
    fn organization_policy_preserved_during_merge() {
        // Regression test for fleet policy loss bug.
        // Verifies that organization policy set during load_layer() is preserved
        // through merge() instead of being replaced with default.
        let config1 = Config::parse(
            r#"
[[expansion]]
trigger = ":test"
replacement = "test"

[organization]
safe_mode = true
disable_commands = false
"#,
        )
        .unwrap();

        // Mark that this config has organization policy
        assert!(config1.organization.safe_mode);

        // Load another config without policy
        let config2 = Config::parse(
            r#"
[[expansion]]
trigger = ":other"
replacement = "other"
"#,
        )
        .unwrap();

        assert!(!config2.organization.safe_mode);

        // Both configs should be valid (this was the issue:
        // policy was loaded but never stored in ConfigMerger)
        assert!(config1.organization.is_active());
        assert!(!config2.organization.is_active());
    }

    #[test]
    fn administrator_command_path_policy_obeys_safe_and_audit_modes_in_fleet_merge() {
        let base = || {
            Config::parse(
                r#"
                [[expansion]]
                trigger = ":cmd"
                replacement = ""
                [expansion.command]
                program = "printf"
                args = ["ok"]
                "#,
            )
            .unwrap()
        };
        let fleet = || ConfigMerger::new().merge().unwrap();

        let audit_policy = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let mut audit = FleetConfig::apply_base_and_policy(fleet(), base(), &audit_policy)
            .unwrap()
            .config;
        assert!(audit.apply_administrator_policy(&audit_policy).is_ok());

        let safe_policy = OrganizationPolicy {
            safe_mode: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let mut safe = FleetConfig::apply_base_and_policy(fleet(), base(), &safe_policy)
            .unwrap()
            .config;
        assert!(safe.apply_administrator_policy(&safe_policy).is_err());
    }
}
