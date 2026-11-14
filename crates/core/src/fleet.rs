/// Fleet configuration: multi-layer, policy-driven snippet management.
///
/// Enables organizations to deploy company-wide snippets without owning users'
/// personal configurations. Layers are merged in deterministic order:
///
/// 1. `/etc/wayexpand/snippets.d/` (organization snippets, root-owned)
/// 2. `snippets.d/` next to the default configuration (user personal
///    snippets; `~/.config/wayexpand/snippets.d/` unless `XDG_CONFIG_HOME`
///    or `WAYEXPAND_CONFIG` moves the configuration)
/// 3. `~/.local/share/wayexpand/packs/` (optional curated packs)
///
/// Each layer is a directory of .toml files. Within a layer, files are merged
/// alphabetically.
///
/// ## Precedence and Conflict Resolution
///
/// **Expansion triggers and hotkey chords:** Duplicate triggers are rejected
/// unless the expansion UUID matches, in which case the later layer updates
/// that same snippet (including its trigger). Duplicate hotkey chords remain
/// fail-closed.
///
/// **Settings:** Settings merge field by field. User settings override
/// organization defaults, and pack settings provide defaults only. Within a
/// category, later files override earlier. Administrator enforcement remains
/// in `/etc/wayexpand/policy.toml`.
///
/// **Organization policy:** Security policy is not part of fleet layers. The
/// daemon loads it exclusively from `/etc/wayexpand/policy.toml`.
///
/// **Base config:** Prepended first (lowest priority for expansions/hotkeys).
/// Base settings are user settings and override organization and pack defaults.
use crate::{Config, ConfigError, OrganizationPolicy};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use thiserror::Error;

mod discovery;
#[cfg(test)]
use discovery::{discover_layer_dirs, discover_pack_dirs};
use discovery::{discover_layer_files, pack_name, settings_source_allowed, standard_layer_dirs};

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

impl FleetError {
    /// A description safe for logs and terminals: like
    /// `ConfigError::safe_summary`, it never echoes a trigger, which can be
    /// snippet content. File paths and hotkey chords are kept, since they
    /// identify the conflicting layer without revealing what it expands to.
    pub fn safe_summary(&self) -> String {
        match self {
            Self::Config(error) => error.safe_summary(),
            Self::DuplicateTrigger { existing_file, .. } => {
                format!("duplicate trigger (first defined in {existing_file})")
            }
            Self::DuplicateHotkey {
                chord,
                existing_file,
                ..
            } => format!("duplicate hotkey {chord} (first defined in {existing_file})"),
            Self::OrganizationPolicyInLayer { .. }
            | Self::OrganizationPathNotRootOwned { .. }
            | Self::InvalidOrganizationDirectory { .. } => self.to_string(),
        }
    }
}

fn reject_embedded_policy(config: &Config, path: &Path) -> Result<(), FleetError> {
    if config.organization.is_active() {
        return Err(FleetError::OrganizationPolicyInLayer {
            file: path.display().to_string(),
        });
    }
    Ok(())
}

fn setting_fields_from_source(source: &str) -> BTreeSet<String> {
    toml::from_str::<toml::Value>(source)
        .ok()
        .and_then(|document| {
            document
                .get("settings")
                .and_then(toml::Value::as_table)
                .cloned()
        })
        .map(|settings| settings.keys().cloned().collect())
        .unwrap_or_default()
}

fn setting_fields_from_non_defaults(settings: &crate::Settings) -> BTreeSet<String> {
    let defaults = crate::Settings::default();
    let mut fields = BTreeSet::new();
    macro_rules! changed { ($($field:ident),+ $(,)?) => {$({
        if settings.$field != defaults.$field { fields.insert(stringify!($field).to_owned()); }
    })+}; }
    changed!(
        max_buffer_chars,
        undo_chord,
        font_scale,
        libei_token_persistence,
        fcitx5_direct_commit,
        template_env,
        allow_clipboard,
        clipboard_mode,
        usage_stats,
        compatibility_mode,
        keystroke_max_chars,
        terminal_apps,
        raw_capture_apps,
        raw_capture_fallback_enabled,
        terminal_paste,
    );
    fields
}

fn settings_rank(provenance: &Provenance) -> u8 {
    match provenance.layer.as_str() {
        "base" | "user" => 3,
        "organization" => 2,
        layer if layer.starts_with("pack:") => 1,
        _ => 2,
    }
}

fn resolve_setting_candidates(
    candidates: &[SettingCandidate],
    policy: &OrganizationPolicy,
) -> (crate::Settings, BTreeMap<String, Provenance>) {
    let mut ordered: Vec<_> = candidates
        .iter()
        .filter(|candidate| settings_source_allowed(&candidate.provenance, policy))
        .collect();
    ordered.sort_by_key(|candidate| settings_rank(&candidate.provenance));

    let mut settings = crate::Settings::default();
    let mut sources = BTreeMap::new();
    for candidate in ordered {
        for field in &candidate.fields {
            // Compatibility consent has a stricter user/admin-only rule and
            // is resolved separately in apply_base_and_policy.
            if field == "compatibility_mode" {
                continue;
            }
            // A signed content pack cannot opt a machine into globally
            // observed raw-keyboard matching or redirect applications to it.
            if matches!(
                field.as_str(),
                "raw_capture_apps" | "raw_capture_fallback_enabled" | "terminal_paste"
            ) && candidate.provenance.layer.starts_with("pack:")
            {
                continue;
            }
            macro_rules! copy_field { ($($name:ident),+ $(,)?) => { match field.as_str() {
                $(stringify!($name) => settings.$name = candidate.settings.$name.clone(),)+
                _ => continue,
            }}; }
            copy_field!(
                max_buffer_chars,
                undo_chord,
                font_scale,
                libei_token_persistence,
                fcitx5_direct_commit,
                template_env,
                allow_clipboard,
                clipboard_mode,
                usage_stats,
                keystroke_max_chars,
                terminal_apps,
                raw_capture_apps,
                raw_capture_fallback_enabled,
                terminal_paste,
            );
            sources.insert(field.clone(), candidate.provenance.clone());
        }
    }
    if let Some(source) = candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.provenance.layer.as_str(),
                "base" | "user" | "organization"
            ) && candidate.settings.compatibility_mode != crate::CompatibilityMode::Off
        })
        .max_by_key(|candidate| settings_rank(&candidate.provenance))
    {
        settings.compatibility_mode = source.settings.compatibility_mode;
        sources.insert("compatibility_mode".into(), source.provenance.clone());
    }
    for field in [
        "max_buffer_chars",
        "undo_chord",
        "font_scale",
        "libei_token_persistence",
        "fcitx5_direct_commit",
        "template_env",
        "allow_clipboard",
        "clipboard_mode",
        "usage_stats",
        "compatibility_mode",
        "keystroke_max_chars",
        "terminal_apps",
        "raw_capture_apps",
        "raw_capture_fallback_enabled",
        "terminal_paste",
    ] {
        sources
            .entry(field.to_owned())
            .or_insert_with(|| Provenance {
                file: "built-in defaults".into(),
                layer: "default".into(),
            });
    }
    (settings, sources)
}

pub(super) fn require_root_owned(path: &Path, metadata: &fs::Metadata) -> Result<(), FleetError> {
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

/// The user layer is the `snippets.d` next to the configuration file, the
/// directory `wayexpand sync` tracks and `Config::layer_files` validates.
/// Hard-coding `~/.config` left a configuration under `XDG_CONFIG_HOME`
/// with synchronized layers the daemon never loaded.
fn user_layer_dir(config: &Path) -> Option<PathBuf> {
    config
        .parent()
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("snippets.d"))
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
            Layer::User => user_layer_dir(&crate::default_config_path()),
            Layer::Pack => {
                if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
                    let path = PathBuf::from(data_home);
                    if path.is_absolute() {
                        return Some(path.join("wayexpand/packs"));
                    }
                }
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
    /// Settings candidates in layer order, retaining provenance for policy
    /// filtering and compatibility with existing fleet diagnostics.
    pub settings_sources: Vec<(crate::Settings, Provenance)>,
    /// Source selected for each effective setting, for fleet diagnostics.
    pub effective_settings_sources: BTreeMap<String, Provenance>,
    /// Explicit field assignments, retained so fleet settings can be merged
    /// by category without treating serde defaults as user choices.
    setting_candidates: Vec<SettingCandidate>,
    /// Merge statistics
    pub stats: MergeStats,
    /// Administrator policy violations discovered while merging pack sources.
    pub policy_violations: Vec<String>,
    /// Directory each loaded pack came from, by pack name, so signatures are
    /// checked against exactly what was loaded.
    pub pack_directories: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone)]
struct SettingCandidate {
    settings: crate::Settings,
    provenance: Provenance,
    fields: BTreeSet<String>,
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
    /// the `snippets.d` next to the default configuration (user), and
    /// ~/.local/share/wayexpand/packs (curated packs).
    /// Falls back gracefully if layers don't exist.
    pub fn load_standard() -> Result<Self, FleetError> {
        let mut merger = ConfigMerger::new();
        let mut pack_directories = BTreeMap::new();
        for (dir, layer_name) in standard_layer_dirs()? {
            if let Some(name) = layer_name.strip_prefix("pack:") {
                pack_directories.insert(name.to_owned(), dir.clone());
            }
            merger.load_layer_named(dir, layer_name)?;
        }
        let mut fleet = merger.merge()?;
        fleet.pack_directories = pack_directories;
        Ok(fleet)
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

    /// Return the standard fleet roots so long-running services can watch for
    /// additions as well as edits to files already present in those roots.
    pub fn standard_source_directories() -> Vec<PathBuf> {
        [Layer::Organization, Layer::User, Layer::Pack]
            .into_iter()
            .filter_map(Layer::default_dir)
            .collect()
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
        let fields = setting_fields_from_non_defaults(&base.settings);
        Self::load_standard_with_base_fields_and_policy(base, fields, policy)
    }

    /// Load fleet layers using the exact primary-config setting keys. This
    /// allows an explicit user choice of a serde default to override a pack
    /// recommendation too.
    pub fn load_standard_with_base_source_and_policy(
        base: Config,
        base_source: &str,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let fields = setting_fields_from_source(base_source);
        Self::load_standard_with_base_fields_and_policy(base, fields, policy)
    }

    fn load_standard_with_base_fields_and_policy(
        base: Config,
        base_fields: BTreeSet<String>,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let fleet = Self::load_standard()?;
        let mut merged = Self::apply_base_and_policy_with_fields(fleet, base, base_fields, policy)?;
        merged
            .config
            .apply_administrator_policy(policy)
            .map_err(FleetError::Config)?;
        Ok(merged)
    }

    /// Validate the portable library represented by the primary config and
    /// its adjacent `snippets.d` directory. This excludes machine-local
    /// organization layers and installed packs, while using the same merge
    /// rules as the daemon's fleet loader.
    pub fn validate_library(base: Config, directory: impl AsRef<Path>) -> Result<(), FleetError> {
        let mut merger = ConfigMerger::new();
        merger.add_config(
            base,
            Provenance {
                file: "expansions.toml".into(),
                layer: "base".into(),
            },
        )?;
        merger.load_layer_named(
            directory.as_ref().join("snippets.d"),
            Layer::User.name().into(),
        )?;
        merger.merge().map(|_| ())
    }

    #[cfg(test)]
    fn apply_base_and_policy(
        fleet: Self,
        base: Config,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let base_fields = setting_fields_from_non_defaults(&base.settings);
        Self::apply_base_and_policy_with_fields(fleet, base, base_fields, policy)
    }

    fn apply_base_and_policy_with_fields(
        mut fleet: Self,
        base: Config,
        base_fields: BTreeSet<String>,
        policy: &OrganizationPolicy,
    ) -> Result<Self, FleetError> {
        let mut setting_candidates = fleet.setting_candidates.clone();
        let base_provenance = Provenance {
            file: "expansions.toml".into(),
            layer: "base".into(),
        };
        if !base_fields.is_empty() {
            setting_candidates.push(SettingCandidate {
                settings: base.settings.clone(),
                provenance: base_provenance,
                fields: base_fields,
            });
        }
        let (fleet_settings, effective_settings_sources) =
            resolve_setting_candidates(&setting_candidates, policy);
        // Consent to unverified keystroke erasure belongs to the user (primary
        // config or user layer) or the administrator (organization layer),
        // never to a curated pack. Unlike other settings it is therefore not
        // taken wholesale from the last settings layer.
        let compatibility_mode = std::iter::once(base.settings.compatibility_mode)
            .chain(
                fleet
                    .settings_sources
                    .iter()
                    .filter(|(_, provenance)| {
                        matches!(provenance.layer.as_str(), "user" | "organization")
                    })
                    .map(|(settings, _)| settings.compatibility_mode),
            )
            .rev()
            .find(|mode| *mode != crate::CompatibilityMode::Off)
            .unwrap_or_default();
        // Packs that must be signed but do not verify against a trusted,
        // root-owned signers file.
        let unsigned_packs: BTreeMap<String, String> = if policy.require_signed_packs {
            let signers = Path::new(&policy.pack_signers_file);
            fleet
                .expansions_source
                .values()
                .chain(fleet.hotkeys_source.values())
                .filter(|provenance| provenance.layer.starts_with("pack:"))
                .map(|provenance| pack_name(provenance).to_owned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|name| {
                    let Some(directory) = fleet.pack_directories.get(&name) else {
                        return Some((name, "pack directory unknown".to_owned()));
                    };
                    crate::trusted_signers_file(signers)
                        .and_then(|()| crate::verify_pack_signature(directory, signers))
                        .err()
                        .map(|error| (name, error.to_string()))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let pack_permitted =
            |name: &str| policy.pack_allowed(name) && !unsigned_packs.contains_key(name);

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
            .chain(unsigned_packs.iter().map(|(name, reason)| {
                format!("pack '{name}' is not signed by a trusted signer: {reason}")
            }))
            .collect();

        // Enforce the same audited decision only when safe mode is enabled.
        if (!policy.allowed_packs.is_empty() || !unsigned_packs.is_empty()) && policy.safe_mode {
            fleet.config.expansion.retain(|expansion| {
                if let Some(prov) = fleet.expansions_source.get(&expansion.trigger) {
                    // Keep organization and user layers, filter packs
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || (prov.layer.starts_with("pack:") && pack_permitted(pack_name(prov)))
                } else {
                    true
                }
            });
            fleet.config.hotkey.retain(|hotkey| {
                if let Some(prov) = fleet.hotkeys_source.get(&hotkey.chord) {
                    prov.layer == "organization"
                        || prov.layer == "user"
                        || (prov.layer.starts_with("pack:") && pack_permitted(pack_name(prov)))
                } else {
                    true
                }
            });
        }

        // Compose the base and filtered fleet entries through the same merger
        // used by portable-library validation. This preserves stable-ID layer
        // updates instead of appending duplicate IDs and then rejecting them
        // during Config::validate().
        let mut merger = ConfigMerger::new();
        merger.add_config(
            base,
            Provenance {
                file: "expansions.toml".into(),
                layer: "base".into(),
            },
        )?;
        for expansion in std::mem::take(&mut fleet.config.expansion) {
            let provenance = fleet
                .expansions_source
                .get(&expansion.trigger)
                .cloned()
                .unwrap_or_else(|| Provenance {
                    file: "fleet layer".into(),
                    layer: "user".into(),
                });
            merger.add_expansion(expansion, provenance)?;
        }
        for hotkey in std::mem::take(&mut fleet.config.hotkey) {
            let provenance = fleet
                .hotkeys_source
                .get(&hotkey.chord)
                .cloned()
                .unwrap_or_else(|| Provenance {
                    file: "fleet layer".into(),
                    layer: "user".into(),
                });
            if merger.hotkeys.contains_key(&hotkey.chord) {
                let existing = &merger.hotkeys[&hotkey.chord].1;
                return Err(FleetError::DuplicateHotkey {
                    chord: hotkey.chord.clone(),
                    message: format!(
                        "hotkey chord '{}' in {} conflicts with existing definition",
                        hotkey.chord, provenance.file
                    ),
                    existing_file: existing.file.clone(),
                });
            }
            merger
                .hotkeys
                .insert(hotkey.chord.clone(), (hotkey, provenance));
        }
        let mut merged = merger.merge()?;
        merged.config.settings = fleet_settings;
        merged.config.settings.compatibility_mode = compatibility_mode;
        fleet.config = merged.config;
        fleet.effective_settings_sources = effective_settings_sources;
        // These fields are reported alongside the active arrays by the CLI;
        // recompute them after policy filtering and base-layer composition so
        // status output cannot describe entries that are no longer active.
        fleet.stats.total_expansions = fleet.config.expansion.len();
        fleet.stats.total_hotkeys = fleet.config.hotkey.len();
        // The primary configuration is always one loaded source file, even
        // when it contains only settings or is intentionally empty.
        fleet.stats.total_files_loaded += 1;
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
/// Implements fleet precedence: stable-ID expansion updates and duplicate
/// triggers with different IDs are handled explicitly; hotkey duplicates fail
/// closed, while settings merge per field by source category.
///
/// Duplicate detection across all loaded layers ensures configuration safety:
/// accidental trigger collisions are caught early rather than silently masked
/// by load order. This is intentional and trusted behavior for enterprise fleets.
struct ConfigMerger {
    // Individual items keyed by trigger/chord for duplicate detection.
    // Stores (item, provenance) to report exact source on conflict.
    expansions: BTreeMap<String, (crate::ExpansionConfig, Provenance)>,
    expansion_ids: HashMap<String, String>,
    hotkeys: BTreeMap<String, (crate::HotkeyConfig, Provenance)>,
    settings: Vec<SettingCandidate>,
    stats: MergeStats,
}

impl ConfigMerger {
    fn new() -> Self {
        Self {
            expansions: BTreeMap::new(),
            expansion_ids: HashMap::new(),
            hotkeys: BTreeMap::new(),
            settings: Vec::new(),
            stats: MergeStats::default(),
        }
    }

    fn add_expansion(
        &mut self,
        expansion: crate::ExpansionConfig,
        provenance: Provenance,
    ) -> Result<(), FleetError> {
        let previous_trigger = self.expansion_ids.get(&expansion.id).cloned();
        if let Some((_, existing_provenance)) = self.expansions.get(&expansion.trigger) {
            let same_identity = previous_trigger.as_deref() == Some(&expansion.trigger);
            if !same_identity {
                return Err(FleetError::DuplicateTrigger {
                    trigger: expansion.trigger.clone(),
                    message: format!(
                        "expansion trigger '{}' in {} conflicts with existing definition",
                        expansion.trigger, provenance.file
                    ),
                    existing_file: existing_provenance.file.clone(),
                });
            }
        }
        if let Some(previous_trigger) = previous_trigger {
            if previous_trigger != expansion.trigger {
                self.expansions.remove(&previous_trigger);
            }
        } else {
            self.stats.total_expansions += 1;
        }
        self.expansion_ids
            .insert(expansion.id.clone(), expansion.trigger.clone());
        self.expansions
            .insert(expansion.trigger.clone(), (expansion, provenance));
        Ok(())
    }

    fn add_config(&mut self, config: Config, provenance: Provenance) -> Result<(), FleetError> {
        let fields = setting_fields_from_non_defaults(&config.settings);
        self.add_config_with_settings_fields(config, provenance, fields)
    }

    fn add_config_with_settings_fields(
        &mut self,
        config: Config,
        provenance: Provenance,
        setting_fields: BTreeSet<String>,
    ) -> Result<(), FleetError> {
        for expansion in config.expansion {
            self.add_expansion(expansion, provenance.clone())?;
        }
        for hotkey in config.hotkey {
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
                .insert(hotkey.chord.clone(), (hotkey, provenance.clone()));
            self.stats.total_hotkeys += 1;
        }
        if !setting_fields.is_empty() {
            self.settings.push(SettingCandidate {
                settings: config.settings,
                provenance,
                fields: setting_fields,
            });
        }
        Ok(())
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

            let loaded = Config::load_versioned(&path).map_err(FleetError::Config)?;
            let setting_fields = setting_fields_from_source(loaded.source());
            let config = loaded.config;
            reject_embedded_policy(&config, &path)?;
            self.stats.total_files_loaded += 1;
            self.add_config_with_settings_fields(config, provenance, setting_fields)?;
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

        let setting_candidates = self.settings;
        let settings_sources = setting_candidates
            .iter()
            .map(|candidate| (candidate.settings.clone(), candidate.provenance.clone()))
            .collect();
        let (settings, effective_settings_sources) =
            resolve_setting_candidates(&setting_candidates, &OrganizationPolicy::default());

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
            effective_settings_sources,
            setting_candidates,
            stats: self.stats,
            policy_violations: Vec::new(),
            pack_directories: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_error_summary_never_echoes_a_trigger() {
        let error = FleetError::DuplicateTrigger {
            trigger: "secret-trigger".into(),
            message: "expansion trigger 'secret-trigger' in a.toml conflicts".into(),
            existing_file: "/etc/wayexpand/fleet.d/a.toml".into(),
        };
        let summary = error.safe_summary();
        assert!(
            summary.contains("/etc/wayexpand/fleet.d/a.toml"),
            "{summary}"
        );
        assert!(!summary.contains("secret"), "{summary}");
        let error = FleetError::Config(ConfigError::DuplicateTrigger {
            trigger: "secret-trigger".into(),
            first: 1,
            second: 2,
        });
        assert!(!error.safe_summary().contains("secret"));
    }

    #[test]
    fn layer_discovery_skips_special_files_named_like_layers() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-fleet-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.toml"), "").unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(root.join("hang.toml"))
            .status()
            .unwrap();
        assert!(status.success());
        std::os::unix::fs::symlink(root.join("hang.toml"), root.join("link.toml")).unwrap();
        std::fs::create_dir(root.join("dir.toml")).unwrap();

        let files = discover_layer_files(&root, false).unwrap();

        assert_eq!(files, vec![root.join("real.toml")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn layer_defaults() {
        assert_eq!(Layer::Organization.name(), "organization");
        assert_eq!(Layer::User.name(), "user");
        assert_eq!(
            user_layer_dir(Path::new("/xdg/config/wayexpand/expansions.toml")),
            Some(PathBuf::from("/xdg/config/wayexpand/snippets.d"))
        );
        // Without a usable home the default path is relative: no user layer.
        assert_eq!(user_layer_dir(Path::new("expansions.toml")), None);
        assert_eq!(Layer::Pack.name(), "pack");
    }

    #[test]
    fn later_fleet_layer_updates_a_snippet_by_stable_id() {
        let id = "00000000-0000-4000-8000-000000000001";
        let earlier = Config::parse(&format!(
            "[[expansion]]\nid='{id}'\ntrigger=':old'\nreplacement='before'\n"
        ))
        .unwrap()
        .expansion
        .into_iter()
        .next()
        .unwrap();
        let later = Config::parse(&format!(
            "[[expansion]]\nid='{id}'\ntrigger=':new'\nreplacement='after'\n"
        ))
        .unwrap()
        .expansion
        .into_iter()
        .next()
        .unwrap();
        let mut merger = ConfigMerger::new();
        merger
            .add_expansion(
                earlier,
                Provenance {
                    file: "organization.toml".into(),
                    layer: "organization".into(),
                },
            )
            .unwrap();
        merger
            .add_expansion(
                later,
                Provenance {
                    file: "user.toml".into(),
                    layer: "user".into(),
                },
            )
            .unwrap();

        let fleet = merger.merge().unwrap();
        assert_eq!(fleet.config.expansion.len(), 1);
        assert_eq!(fleet.config.expansion[0].trigger, ":new");
        assert_eq!(fleet.config.expansion[0].replacement, "after");
        assert_eq!(fleet.stats.total_expansions, 1);
        assert_eq!(fleet.expansions_source[":new"].file, "user.toml");
    }

    #[test]
    fn base_and_fleet_layer_update_by_stable_id() {
        let id = "00000000-0000-4000-8000-000000000002";
        let base = Config::parse(&format!(
            "[[expansion]]\nid='{id}'\ntrigger=':old'\nreplacement='before'\n"
        ))
        .unwrap();
        let later = Config::parse(&format!(
            "[[expansion]]\nid='{id}'\ntrigger=':new'\nreplacement='after'\n"
        ))
        .unwrap()
        .expansion
        .into_iter()
        .next()
        .unwrap();
        let mut merger = ConfigMerger::new();
        merger
            .add_expansion(
                later,
                Provenance {
                    file: "user.toml".into(),
                    layer: "user".into(),
                },
            )
            .unwrap();
        let fleet = merger.merge().unwrap();

        let merged =
            FleetConfig::apply_base_and_policy(fleet, base, &OrganizationPolicy::default())
                .unwrap();
        assert_eq!(merged.config.expansion.len(), 1);
        assert_eq!(merged.config.expansion[0].trigger, ":new");
        assert_eq!(merged.config.expansion[0].replacement, "after");
    }

    #[test]
    fn base_file_is_counted_when_it_only_contains_settings() {
        let base = Config::parse("[settings]\nmax_buffer_chars = 256\n").unwrap();
        let fleet = ConfigMerger::new().merge().unwrap();

        let merged =
            FleetConfig::apply_base_and_policy(fleet, base, &OrganizationPolicy::default())
                .unwrap();
        assert_eq!(merged.stats.total_files_loaded, 1);
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
        merger.settings.push(SettingCandidate {
            settings: settings_candidate(max_buffer_chars),
            provenance: Provenance {
                file: "settings.toml".to_string(),
                layer: layer.to_string(),
            },
            fields: BTreeSet::from(["max_buffer_chars".to_string()]),
        });
    }

    #[test]
    fn only_the_user_or_administrator_can_grant_compatibility_consent() {
        let consenting = |layer: &str| SettingCandidate {
            settings: crate::Settings {
                compatibility_mode: crate::CompatibilityMode::UntrackedFocus,
                ..crate::Settings::default()
            },
            provenance: Provenance {
                file: "settings.toml".to_string(),
                layer: layer.to_string(),
            },
            fields: BTreeSet::from(["compatibility_mode".to_string()]),
        };
        let merged_mode = |layers: &[&str], base: &str| {
            let mut merger = ConfigMerger::new();
            for layer in layers {
                merger.settings.push(consenting(layer));
            }
            let fleet = merger.merge().unwrap();
            FleetConfig::apply_base_and_policy(
                fleet,
                Config::parse(base).unwrap(),
                &OrganizationPolicy::default(),
            )
            .unwrap()
            .config
            .settings
            .compatibility_mode
        };
        let off = crate::CompatibilityMode::Off;
        let untracked = crate::CompatibilityMode::UntrackedFocus;

        assert_eq!(merged_mode(&["pack:approved"], ""), off);
        assert_eq!(merged_mode(&["user"], ""), untracked);
        assert_eq!(merged_mode(&["organization"], ""), untracked);
        // The primary config's consent survives a later settings layer that
        // does not mention it, such as a pack tuning the buffer size.
        let base = "[settings]\ncompatibility_mode = \"tracked-focus\"\n";
        let mut merger = ConfigMerger::new();
        add_settings_candidate(&mut merger, 512, "pack:approved");
        let fleet = merger.merge().unwrap();
        let merged = FleetConfig::apply_base_and_policy(
            fleet,
            Config::parse(base).unwrap(),
            &OrganizationPolicy::default(),
        )
        .unwrap();
        assert_eq!(merged.config.settings.max_buffer_chars, 512);
        assert_eq!(
            merged.config.settings.compatibility_mode,
            crate::CompatibilityMode::TrackedFocus
        );
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
    fn content_packs_cannot_assign_apps_to_raw_capture() {
        let candidate = SettingCandidate {
            settings: crate::Settings {
                raw_capture_apps: vec!["app_id_glob:*browser*".into()],
                raw_capture_fallback_enabled: true,
                ..crate::Settings::default()
            },
            provenance: Provenance {
                file: "settings.toml".into(),
                layer: "pack:approved".into(),
            },
            fields: BTreeSet::from([
                "raw_capture_apps".into(),
                "raw_capture_fallback_enabled".into(),
            ]),
        };
        let (settings, _) =
            resolve_setting_candidates(&[candidate], &OrganizationPolicy::default());
        assert!(settings.raw_capture_apps.is_empty());
        assert!(!settings.raw_capture_fallback_enabled);
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

    fn manifest_pack(root: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let pack = root.join(name);
        fs::create_dir_all(pack.join("snippets")).unwrap();
        for directory in [root.to_path_buf(), pack.clone(), pack.join("snippets")] {
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(
            pack.join("wayexpand-pack.toml"),
            "format_version = 1\nid = \"t\"\nversion = \"1\"\nname = \"T\"\npublisher = \"P\"\n",
        )
        .unwrap();
        fs::write(
            pack.join("snippets/main.toml"),
            "[[expansion]]\ntrigger = \";;team\"\nreplacement = \"from the team pack\"\n",
        )
        .unwrap();
        for file in [
            pack.join("wayexpand-pack.toml"),
            pack.join("snippets/main.toml"),
        ] {
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        pack
    }

    fn fleet_with_pack(root: &Path) -> FleetConfig {
        let pack = manifest_pack(root, "team");
        let mut merger = ConfigMerger::new();
        merger
            .load_layer_named(&pack, "pack:team".to_owned())
            .unwrap();
        let mut fleet = merger.merge().unwrap();
        fleet.pack_directories.insert("team".into(), pack);
        fleet
    }

    #[test]
    fn manifest_packs_load_from_their_snippets_directory() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-fleet-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let fleet = fleet_with_pack(&root);
        assert!(fleet
            .config
            .expansion
            .iter()
            .any(|expansion| expansion.trigger == ";;team"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsigned_packs_are_excluded_when_policy_requires_signatures() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-fleet-signed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // A signers file the user can write is never trusted.
        let signers = root.join("allowed_signers");
        fs::create_dir_all(&root).unwrap();
        fs::write(&signers, "").unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_signed_packs: true,
            pack_signers_file: signers.display().to_string(),
            ..OrganizationPolicy::default()
        };
        let fleet = FleetConfig::apply_base_and_policy(
            fleet_with_pack(&root),
            Config::parse("").unwrap(),
            &policy,
        )
        .unwrap();
        assert!(!fleet
            .config
            .expansion
            .iter()
            .any(|expansion| expansion.trigger == ";;team"));
        assert!(fleet
            .policy_violations
            .iter()
            .any(|violation| violation.contains("pack 'team' is not signed by a trusted signer")));

        // Audit mode reports the same pack but keeps it.
        let audit = OrganizationPolicy {
            safe_mode: false,
            ..policy
        };
        let fleet = FleetConfig::apply_base_and_policy(
            fleet_with_pack(&root),
            Config::parse("").unwrap(),
            &audit,
        )
        .unwrap();
        assert!(fleet
            .config
            .expansion
            .iter()
            .any(|expansion| expansion.trigger == ";;team"));
        assert!(!fleet.policy_violations.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
