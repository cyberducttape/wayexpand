use anyhow::Result;
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
use tracing::{error, info};
use wayexpand_core::{Config, ConfigError, ExpansionEngine, FleetConfig, Layer};

const MAX_CONSISTENCY_ATTEMPTS: usize = 3;
const FINGERPRINT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub struct ReloadableConfig {
    path: PathBuf,
    stamp: Option<FileStamp>,
    observed: Option<FileStamp>,
    last_fingerprint_check: Option<Instant>,
    last_metadata: Option<MetadataStamp>,
    fleet: bool,
    fleet_signature: u64,
    last_fleet_check: Instant,
    pub engine: ExpansionEngine,
    healthy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    length: u64,
    inode: u64,
    change_time: i64,
    change_time_nsec: i64,
    fingerprint: u64,
}

/// Cheap metadata-only stamp (without reading content or computing fingerprint).
/// Used for fast change detection before doing expensive file reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MetadataStamp {
    modified: SystemTime,
    length: u64,
    inode: u64,
    change_time: i64,
    change_time_nsec: i64,
}

/// Fast metadata-only check without reading file contents. Used to detect if
/// a full file_stamp() call is needed. This avoids reading large configs when
/// metadata hasn't changed.
fn metadata_stamp(path: &Path) -> Option<MetadataStamp> {
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let file = fs::File::from(descriptor);
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let length = metadata.len();
    let inode = metadata.ino();
    let change_time = metadata.ctime();
    let change_time_nsec = metadata.ctime_nsec();
    Some(MetadataStamp {
        modified,
        length,
        inode,
        change_time,
        change_time_nsec,
    })
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    // Open nonblocking and validate the resulting descriptor. The metadata
    // check above is only an optimization; a path can be replaced between
    // that check and the open, and a FIFO must never stall the reload loop.
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let file = fs::File::from(descriptor);
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let length = metadata.len();
    let inode = metadata.ino();
    let change_time = metadata.ctime();
    let change_time_nsec = metadata.ctime_nsec();
    let mut contents = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut contents)
        .ok()?;
    // Use a stable hash algorithm independent of Rust compiler version.
    // A simple byte-wise XOR and sum is sufficient to detect content changes
    // on filesystems with coarse timestamps (same-size edits).
    let mut fingerprint: u64 = 0;
    for chunk in contents.chunks(8) {
        let mut bytes = [0_u8; 8];
        bytes[..chunk.len()].copy_from_slice(chunk);
        fingerprint = fingerprint.wrapping_add(u64::from_le_bytes(bytes));
    }
    Some(FileStamp {
        modified,
        length,
        inode,
        change_time,
        change_time_nsec,
        fingerprint,
    })
}

impl ReloadableConfig {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        Self::load_mode(path.into(), false)
    }

    pub fn load_with_fleet(path: impl Into<PathBuf>) -> Result<Self> {
        Self::load_mode(path.into(), true)
    }

    fn load_mode(path: PathBuf, fleet: bool) -> Result<Self> {
        let (config, stamp) = load_for_mode(&path, fleet)?;
        let engine = ExpansionEngine::new(config)
            .map_err(|error| anyhow::anyhow!("invalid configuration: {error}"))?;
        let last_metadata = stamp.map(|s| MetadataStamp {
            modified: s.modified,
            length: s.length,
            inode: s.inode,
            change_time: s.change_time,
            change_time_nsec: s.change_time_nsec,
        });
        Ok(Self {
            path,
            stamp,
            observed: stamp,
            // Force a content fingerprint on the first polling cycle. This
            // catches same-size edits on filesystems with coarse timestamps.
            last_fingerprint_check: None,
            last_metadata,
            fleet,
            fleet_signature: standard_fleet_signature(),
            last_fleet_check: Instant::now(),
            engine,
            healthy: true,
        })
    }

    /// Parse first, then replace the live engine. Invalid edits leave the old
    /// configuration running and are reported to the operator.
    pub fn reload_if_changed(&mut self) {
        let current = self.poll_stamp();
        let fleet_changed =
            self.fleet && self.last_fleet_check.elapsed() >= FINGERPRINT_REFRESH_INTERVAL && {
                self.last_fleet_check = Instant::now();
                standard_fleet_signature() != self.fleet_signature
            };
        if current == self.observed && !fleet_changed {
            return;
        }
        self.observed = current;
        self.reload_current(current);
    }

    pub fn reload_now(&mut self) {
        let current = file_stamp(&self.path);
        self.last_fingerprint_check = Some(Instant::now());
        self.last_metadata = current.map(|s| MetadataStamp {
            modified: s.modified,
            length: s.length,
            inode: s.inode,
            change_time: s.change_time,
            change_time_nsec: s.change_time_nsec,
        });
        self.observed = current;
        self.reload_current(current);
    }

    fn reload_current(&mut self, current: Option<FileStamp>) {
        match load_for_mode(&self.path, self.fleet) {
            Ok((config, stable_stamp)) => {
                let count = config.expansion.len();
                match ExpansionEngine::new(config) {
                    Ok(mut engine) => {
                        if self.engine.async_commands_enabled() {
                            engine.enable_async_commands();
                        }
                        // A fresh engine has no window context yet. Without
                        // this, any reload (e.g. every GUI save) would
                        // wrongly fail-close `app_filter`-scoped expansions
                        // until the next real focus change, even though the
                        // user's actual window never changed.
                        engine.set_current_window(self.engine.current_window().cloned());
                        // CRITICAL: Restore runtime safety state across reloads.
                        // A fresh engine defaults user_paused=false and sensitive_focus=false,
                        // losing any protection or pause state. This causes:
                        // - Password-field protection to be lost until the next compositor
                        //   focus event, creating a security window where matching resumes
                        //   in a sensitive field despite the old engine being paused.
                        // - User pause state to be lost, making the daemon appear to resume
                        //   matching even though the control status still says paused.
                        engine.set_user_paused(self.engine.is_user_paused());
                        engine.set_sensitive_focus(self.engine.is_sensitive_focus());
                        self.engine = engine;
                        self.stamp = stable_stamp;
                        self.observed = stable_stamp;
                        self.last_fingerprint_check = Some(Instant::now());
                        self.fleet_signature = standard_fleet_signature();
                        self.healthy = true;
                        info!(expansions = count, "configuration reloaded");
                    }
                    Err(error) => {
                        self.healthy = false;
                        error!(
                            reason = %safe_reload_error(&anyhow::Error::new(error)),
                            "configuration reload rejected; keeping previous configuration"
                        )
                    }
                }
            }
            Err(error) => {
                self.healthy = false;
                // Keep the pre-read stamp as the observed value. If the file
                // changed while it was being read, the next polling cycle
                // must retry instead of considering the unstable read settled.
                self.observed = current;
                error!(
                    reason = %safe_reload_error(&error),
                    "configuration reload rejected; keeping previous configuration"
                )
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn healthy(&self) -> bool {
        self.healthy
    }

    /// Check for file changes efficiently. First checks metadata only (cheap),
    /// then only does full read-and-hash if metadata changed or if it's been
    /// a full FINGERPRINT_REFRESH_INTERVAL since the last content check.
    /// This avoids repeatedly reading/hashing large configs when they haven't
    /// actually changed.
    fn poll_stamp(&mut self) -> Option<FileStamp> {
        let current_metadata = metadata_stamp(&self.path)?;

        // If metadata hasn't changed since last check, return cached stamp
        if let Some(last) = self.last_metadata {
            if current_metadata == last {
                return self.observed;
            }
        }

        // Metadata changed, or this is the first check. Now do full content hash.
        // But still rate-limit full reads if metadata keeps changing without
        // content actually changing (noisy filesystem operations).
        let too_soon = self
            .last_fingerprint_check
            .is_some_and(|checked| checked.elapsed() < FINGERPRINT_REFRESH_INTERVAL);
        if too_soon {
            // Metadata changed but we're still in rate-limit window. This
            // can happen with editors that touch mtime repeatedly. Return
            // the observed stamp and retry next interval.
            return self.observed;
        }

        let current = file_stamp(&self.path);
        self.last_fingerprint_check = Some(Instant::now());
        self.last_metadata = current.map(|s| MetadataStamp {
            modified: s.modified,
            length: s.length,
            inode: s.inode,
            change_time: s.change_time,
            change_time_nsec: s.change_time_nsec,
        });
        current
    }
}

fn safe_reload_error(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<ConfigError>() {
        return error.safe_summary();
    }
    "configuration could not be read consistently".into()
}

fn load_consistent(path: &Path) -> Result<(Config, Option<FileStamp>)> {
    for _attempt in 0..MAX_CONSISTENCY_ATTEMPTS {
        let before = file_stamp(path);
        let config = Config::load(path)?;
        let after = file_stamp(path);
        if before == after {
            return Ok((config, after));
        }
    }
    anyhow::bail!(
        "configuration changed while being read after {MAX_CONSISTENCY_ATTEMPTS} attempts"
    )
}

fn load_for_mode(path: &Path, fleet: bool) -> Result<(Config, Option<FileStamp>)> {
    let (base, stamp) = load_consistent(path)?;
    if fleet {
        let merged = FleetConfig::load_standard_with_base(base)
            .map_err(|error| anyhow::anyhow!("fleet configuration invalid: {error}"))?;
        Ok((merged.config, stamp))
    } else {
        Ok((base, stamp))
    }
}

fn standard_fleet_signature() -> u64 {
    let mut hasher = DefaultHasher::new();
    for (_, directory) in [Layer::Organization, Layer::User, Layer::Pack]
        .into_iter()
        .filter_map(|layer| layer.default_dir().map(|directory| (layer, directory)))
    {
        directory.hash(&mut hasher);
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .collect();
        paths.sort();
        for path in paths {
            path.hash(&mut hasher);
            if let Ok(contents) = fs::read(&path) {
                contents.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };
    use wayexpand_core::InputEvent;

    fn temporary_config() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("wayexpand-reload-test-{nonce}.toml"))
    }

    fn config_text(replacement: &str) -> String {
        format!("[[expansion]]\ntrigger = \":x\"\nreplacement = {replacement:?}\n")
    }

    /// Writes a fixture with an explicit private mode. Relying on the
    /// ambient umask fails under a default of 002 (Debian/Ubuntu
    /// user-private-group setups), where the file lands group-writable 0664
    /// and `Config::load` correctly refuses to load it.
    fn write_config(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn reload_preserves_window_context_for_app_filtered_expansions() {
        use wayexpand_core::WindowContext;

        let path = temporary_config();
        write_config(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [\"kate\"]\n",
        );
        let mut config = ReloadableConfig::load(&path).unwrap();
        config.engine.set_current_window(Some(WindowContext {
            app_id: Some("org.kde.kate".into()),
            title: None,
        }));

        // Any reload -- including one an unrelated GUI edit would trigger --
        // must not forget the window the user is actually still in.
        write_config(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"z\"\napp_filter = [\"kate\"]\n",
        );
        config.reload_now();
        assert!(config.healthy());

        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "z");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn valid_reload_replaces_active_engine() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config = ReloadableConfig::load(&path).unwrap();
        write_config(&path, &config_text("new replacement"));
        config.reload_now();
        assert!(config.healthy());

        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "new replacement");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_reload_keeps_previous_engine() {
        let path = temporary_config();
        write_config(&path, &config_text("stable"));
        let mut config = ReloadableConfig::load(&path).unwrap();
        write_config(&path, "[[expansion]]\ntrigger = ");
        config.reload_now();
        assert!(!config.healthy());

        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "stable");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_keeps_previous_engine_and_reloads_when_restored() {
        let path = temporary_config();
        write_config(&path, &config_text("before outage"));
        let mut config = ReloadableConfig::load(&path).unwrap();
        fs::remove_file(&path).unwrap();
        config.reload_now();
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "before outage");

        write_config(&path, &config_text("after restore"));
        config.reload_now();
        assert!(config.healthy());
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "after restore");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn in_place_same_size_edit_is_detected() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config = ReloadableConfig::load(&path).unwrap();
        write_config(&path, &config_text("new"));
        config.reload_if_changed();

        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn consistent_load_returns_the_stamp_for_the_loaded_file() {
        let path = temporary_config();
        write_config(&path, &config_text("stable read"));

        let (_config, stamp) = load_consistent(&path).unwrap();
        assert_eq!(stamp, file_stamp(&path));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn unchanged_metadata_reuses_existing_fingerprint() {
        let path = temporary_config();
        write_config(&path, &config_text("stable metadata"));
        let mut config = ReloadableConfig::load(&path).unwrap();
        let stamp = config.observed.unwrap();
        config.last_fingerprint_check = Some(Instant::now());
        let reused = config.poll_stamp().unwrap();
        assert_eq!(reused, stamp);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_preserves_runtime_safety_state() {
        // CRITICAL P0 SECURITY TEST: Verify that config reloads do NOT
        // lose sensitive_focus and user_paused state.
        //
        // A fresh ExpansionEngine defaults both to false, meaning:
        // - If sensitive_focus was true (password field), a reload would
        //   resume matching in that field until the next compositor event.
        // - If user_paused was true, a reload would appear to resume matching
        //   despite the control API still reporting pause.
        //
        // Both are critical for maintaining password-field protection.
        let path = temporary_config();
        write_config(&path, &config_text("initial"));
        let mut config = ReloadableConfig::load(&path).unwrap();

        // Simulate entering a sensitive field and pausing the user.
        config.engine.set_sensitive_focus(true);
        config.engine.set_user_paused(true);
        assert!(config.engine.is_sensitive_focus());
        assert!(config.engine.is_user_paused());

        // Trigger a reload (e.g., from a GUI save).
        write_config(&path, &config_text("reloaded"));
        config.reload_now();
        assert!(config.healthy());

        // CRITICAL: These states MUST be preserved across the reload.
        assert!(
            config.engine.is_sensitive_focus(),
            "sensitive_focus lost on reload - password field protection bypassed!"
        );
        assert!(
            config.engine.is_user_paused(),
            "user_paused lost on reload - pause state lost!"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_diagnostics_do_not_echo_configuration_details() {
        let error = anyhow::Error::new(ConfigError::DuplicateTrigger {
            trigger: ":secret-trigger".into(),
            first: 1,
            second: 2,
        });
        let summary = safe_reload_error(&error);
        assert_eq!(summary, "duplicate trigger in expansions 1 and 2");
        assert!(!summary.contains("secret-trigger"));
    }
}
