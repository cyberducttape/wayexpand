use anyhow::Result;
use std::{
    fs,
    hash::{Hash, Hasher},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
use tracing::{error, info};
use wayexpand_core::{Config, ConfigError, ExpansionEngine};

const MAX_CONSISTENCY_ATTEMPTS: usize = 3;
const FINGERPRINT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub struct ReloadableConfig {
    path: PathBuf,
    stamp: Option<FileStamp>,
    observed: Option<FileStamp>,
    last_fingerprint_check: Option<Instant>,
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
        let path = path.into();
        let (config, stamp) = load_consistent(&path)?;
        let engine = ExpansionEngine::new(config)
            .map_err(|error| anyhow::anyhow!("invalid configuration: {error}"))?;
        Ok(Self {
            path,
            stamp,
            observed: stamp,
            // Force a content fingerprint on the first polling cycle. This
            // catches same-size edits on filesystems with coarse timestamps.
            last_fingerprint_check: None,
            engine,
            healthy: true,
        })
    }

    /// Parse first, then replace the live engine. Invalid edits leave the old
    /// configuration running and are reported to the operator.
    pub fn reload_if_changed(&mut self) {
        let current = self.poll_stamp();
        if current == self.observed {
            return;
        }
        self.observed = current;
        self.reload_current(current);
    }

    pub fn reload_now(&mut self) {
        let current = file_stamp(&self.path);
        self.last_fingerprint_check = Some(Instant::now());
        self.observed = current;
        self.reload_current(current);
    }

    fn reload_current(&mut self, current: Option<FileStamp>) {
        match load_consistent(&self.path) {
            Ok((config, stable_stamp)) => {
                let count = config.expansion.len();
                match ExpansionEngine::new(config) {
                    Ok(mut engine) => {
                        // A fresh engine has no window context yet. Without
                        // this, any reload (e.g. every GUI save) would
                        // wrongly fail-close `app_filter`-scoped expansions
                        // until the next real focus change, even though the
                        // user's actual window never changed.
                        engine.set_current_window(self.engine.current_window().cloned());
                        self.engine = engine;
                        self.stamp = stable_stamp;
                        self.observed = stable_stamp;
                        self.last_fingerprint_check = Some(Instant::now());
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

    /// Rate-limited to at most one full read-and-hash per
    /// `FINGERPRINT_REFRESH_INTERVAL`, regardless of how often this is
    /// polled or how often the file's metadata appears to change. A path
    /// whose mtime is touched every poll cycle without content changing
    /// (a noisy watcher, an editor that re-saves repeatedly) must not turn
    /// into a full config read on every call: this runs on the daemon's
    /// main event-processing thread. The tradeoff is that a genuine edit
    /// can take up to one interval longer to be observed.
    fn poll_stamp(&mut self) -> Option<FileStamp> {
        let too_soon = self
            .last_fingerprint_check
            .is_some_and(|checked| checked.elapsed() < FINGERPRINT_REFRESH_INTERVAL);
        if too_soon {
            return self.observed;
        }

        let current = file_stamp(&self.path);
        self.last_fingerprint_check = Some(Instant::now());
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
