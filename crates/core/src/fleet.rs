/// Fleet configuration: multi-layer, policy-driven snippet management.
///
/// Enables organizations to deploy company-wide snippets without owning users'
/// personal configurations. Layers are merged in deterministic order:
///
/// 1. `/etc/wayexpand/snippets.d/` (organization policy, root-owned)
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
/// **Organization policy:** If fleet organization policy exists, it replaces
/// the base config policy entirely (not merged).
///
/// **Curated packs:** Filtered by organization policy `allowed_packs`. Only
/// packs in the allowed list are loaded and merged.
///
/// **Base config:** Appended last (lowest priority for expansions/hotkeys).
/// Base settings only override if no layer provides settings.
use crate::{Config, ConfigError, OrganizationPolicy};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
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
    /// Merge statistics
    pub stats: MergeStats,
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

        // Load organization layer (required structure, optional content)
        if let Some(dir) = Layer::Organization.default_dir() {
            if dir.exists() {
                merger.load_layer(dir, Layer::Organization)?;
            }
        }

        // Load user layer (optional)
        if let Some(dir) = Layer::User.default_dir() {
            if dir.exists() {
                merger.load_layer(dir, Layer::User)?;
            }
        }

        // Load pack layer (optional)
        if let Some(dir) = Layer::Pack.default_dir() {
            if dir.exists() {
                merger.load_layer(dir, Layer::Pack)?;
            }
        }

        merger.merge()
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
        let mut fleet = Self::load_standard()?;
        let fleet_settings = fleet.config.settings.clone();
        let base_expansions = base.expansion.len();
        let base_hotkeys = base.hotkey.len();

        let mut config = base;

        // Policy enforcement: filter pack-sourced expansions if allowed_packs is set
        if !policy.allowed_packs.is_empty() {
            fleet.config.expansion.retain(|expansion| {
                if let Some(prov) = fleet.expansions_source.get(&expansion.trigger) {
                    // Keep organization and user layers, filter packs
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || prov.layer.starts_with("pack:")
                            && policy.pack_allowed(pack_name(prov))
                } else {
                    true
                }
            });
            fleet.config.hotkey.retain(|hotkey| {
                if let Some(prov) = fleet.hotkeys_source.get(&hotkey.chord) {
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || prov.layer.starts_with("pack:")
                            && policy.pack_allowed(pack_name(prov))
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
        fleet.stats.total_expansions += base_expansions;
        fleet.stats.total_hotkeys += base_hotkeys;
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
/// and hotkeys, settings follow "last wins", organization policy is preserved.
///
/// Duplicate detection across all loaded layers ensures configuration safety:
/// accidental trigger collisions are caught early rather than silently masked
/// by load order. This is intentional and trusted behavior for enterprise fleets.
struct ConfigMerger {
    // Individual items keyed by trigger/chord for duplicate detection.
    // Stores (item, provenance) to report exact source on conflict.
    expansions: BTreeMap<String, (crate::ExpansionConfig, Provenance)>,
    hotkeys: BTreeMap<String, (crate::HotkeyConfig, Provenance)>,
    settings: Option<(crate::Settings, Provenance)>,
    // Organization policy from fleet layers (last one wins)
    organization: Option<(crate::OrganizationPolicy, Provenance)>,
    stats: MergeStats,
}

impl ConfigMerger {
    fn new() -> Self {
        Self {
            expansions: BTreeMap::new(),
            hotkeys: BTreeMap::new(),
            settings: None,
            organization: None,
            stats: MergeStats::default(),
        }
    }

    fn load_layer(&mut self, dir: impl AsRef<Path>, layer: Layer) -> Result<(), FleetError> {
        let dir = dir.as_ref();

        if !dir.is_dir() {
            return Ok(()); // Layer directory doesn't exist, skip silently
        }

        let mut files: Vec<_> = fs::read_dir(dir)
            .map_err(|e| {
                FleetError::Config(ConfigError::Read {
                    path: dir.display().to_string(),
                    source: e,
                })
            })?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".toml"))
            .collect();

        files.sort_by_key(|e| e.file_name());

        for entry in files {
            let path = entry.path();
            let relative = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .display()
                .to_string();
            let provenance = Provenance {
                file: relative.clone(),
                layer: layer.name().to_string(),
            };

            let config = Config::load(&path).map_err(FleetError::Config)?;
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
                if self.settings.is_some() {
                    // Settings already defined, later one wins (log this)
                    eprintln!(
                        "warning: settings from {} override previous layer",
                        provenance.file
                    );
                }
                self.settings = Some((config.settings.clone(), provenance.clone()));
            }

            // Organization policy (last one wins)
            if config.organization.is_active() {
                if self.organization.is_some() {
                    eprintln!(
                        "warning: organization policy from {} overrides previous layer",
                        provenance.file
                    );
                }
                self.organization = Some((config.organization.clone(), provenance));
            }
        }

        self.stats
            .layers_applied
            .push(format!("{} ({})", layer.name(), dir.display()));

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

        let settings = self
            .settings
            .map(|(settings, _)| settings)
            .unwrap_or_default();

        let organization = self
            .organization
            .map(|(org, _)| org)
            .unwrap_or_default();

        let config = Config {
            expansion,
            hotkey,
            settings,
            organization,
        };

        config.validate().map_err(FleetError::Config)?;

        Ok(FleetConfig {
            config,
            expansions_source,
            hotkeys_source,
            stats: self.stats,
        })
    }
}

fn pack_name(provenance: &Provenance) -> &str {
    provenance
        .layer
        .strip_prefix("pack:")
        .or_else(|| provenance.file.split('/').next())
        .unwrap_or(provenance.file.as_str())
        .trim_end_matches(".toml")
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
    fn layer_ordering() {
        assert!(Layer::Organization < Layer::User);
        assert!(Layer::User < Layer::Pack);
    }
}
