use crate::{Config, ConfigError};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, PoisonError, RwLock},
    time::SystemTime,
};

const MAX_RELOAD_ATTEMPTS: usize = 3;

/// Shared validated configuration snapshot with generation notifications.
pub struct ConfigStore {
    path: PathBuf,
    snapshot: RwLock<ConfigSnapshot>,
    /// Metadata observed on the most recent poll, whether or not it parsed.
    /// Keeping this separate from `applied_stamp` prevents unchanged invalid
    /// content from being reparsed on every fallback-poll cycle.
    observed_stamp: Mutex<Option<FileStamp>>,
    reload_error: Mutex<Option<String>>,
    subscribers: Mutex<Vec<mpsc::SyncSender<u64>>>,
}

/// A coherent configuration and generation pair. Clone this once when both
/// values must describe the same applied configuration.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub config: Arc<Config>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStoreStatus {
    pub state: &'static str,
    pub error: Option<String>,
    pub generation: u64,
}

impl ConfigStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<Arc<Self>, ConfigError> {
        let path = path.into();
        let loaded = Config::load_versioned(&path)?;
        Ok(Arc::new(Self {
            observed_stamp: Mutex::new(metadata_stamp(&path)),
            path,
            snapshot: RwLock::new(ConfigSnapshot {
                config: Arc::new(loaded.config),
                generation: 0,
            }),
            reload_error: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
        }))
    }
    /// Return only the configuration. Use [`Self::snapshot`] when its
    /// corresponding generation is also needed.
    #[deprecated(note = "use snapshot() to read configuration and generation coherently")]
    pub fn config(&self) -> Arc<Config> {
        self.snapshot().config
    }
    pub fn snapshot(&self) -> ConfigSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// Return only the generation. Use [`Self::snapshot`] when its
    /// corresponding configuration is also needed.
    #[deprecated(note = "use snapshot() to read configuration and generation coherently")]
    pub fn generation(&self) -> u64 {
        self.snapshot().generation
    }
    pub fn status(&self) -> ConfigStoreStatus {
        let error = self
            .reload_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ConfigStoreStatus {
            state: if error.is_some() {
                "reload-error"
            } else {
                "ok"
            },
            error,
            generation: self.snapshot().generation,
        }
    }
    pub fn subscribe(&self) -> mpsc::Receiver<u64> {
        // Notifications are invalidation signals, not a complete change log.
        // A full channel retains its older value; consumers must read
        // snapshot() for the authoritative latest config/generation pair.
        // Capacity one prevents a stalled consumer from retaining history.
        let (sender, receiver) = mpsc::sync_channel(1);
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sender);
        receiver
    }
    pub fn atomically_replace(&self, config: Config) -> u64 {
        let mut snapshot = self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        snapshot.config = Arc::new(config);
        snapshot.generation = snapshot.generation.wrapping_add(1);
        let value = snapshot.generation;
        // Keep replacements and their bounded notifications ordered together.
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|subscriber| match subscriber.try_send(value) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            });
        drop(snapshot);
        value
    }
    /// Invalid edits leave the last valid snapshot active.
    pub fn reload_if_changed(&self) -> Result<bool, ConfigError> {
        let current = metadata_stamp(&self.path);
        let mut observed_stamp = self
            .observed_stamp
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current == *observed_stamp {
            return Ok(false);
        }
        let mut last_error = None;
        for _ in 0..MAX_RELOAD_ATTEMPTS {
            let before = metadata_stamp(&self.path);
            let loaded = match Config::load_versioned(&self.path) {
                Ok(loaded) => loaded,
                Err(error) => {
                    let after = metadata_stamp(&self.path);
                    *observed_stamp = after;
                    last_error = Some(error);
                    if before == after {
                        break;
                    }
                    continue;
                }
            };

            // Re-read through a securely validated descriptor, but compare
            // the exact source instead of parsing and validating the same TOML
            // a second time. This still rejects a concurrent in-place edit or
            // atomic rename before publishing the first parsed snapshot.
            let verified = match Config::source_matches_revision(&self.path, &loaded.revision) {
                Ok(verified) => verified,
                Err(error) => {
                    let after = metadata_stamp(&self.path);
                    *observed_stamp = after;
                    last_error = Some(error);
                    if before == after {
                        break;
                    }
                    continue;
                }
            };
            let after = metadata_stamp(&self.path);
            *observed_stamp = after;
            if before == after && verified {
                self.atomically_replace(loaded.config);
                *self
                    .reload_error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = None;
                return Ok(true);
            }

            last_error = Some(ConfigError::Read {
                path: self.path.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "configuration changed while loading",
                ),
            });
        }

        let error = last_error.unwrap_or_else(|| ConfigError::Read {
            path: self.path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "configuration changed while loading",
            ),
        });
        *self
            .reload_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error.safe_summary());
        Err(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    length: u64,
    inode: u64,
    change_time: i64,
    change_time_nsec: i64,
}

/// Metadata is sufficient to avoid reading the configuration on every poll.
/// Atomic replacements change the inode; ordinary edits update size, mtime, or
/// ctime. Config::load performs the authoritative secure read after a change
/// is observed.
fn metadata_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: metadata.modified().ok(),
        length: metadata.len(),
        inode: metadata.ino(),
        change_time: metadata.ctime(),
        change_time_nsec: metadata.ctime_nsec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{
            atomic::{AtomicBool, Ordering},
            Barrier,
        },
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn write_private(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn atomic_write_private(path: &Path, contents: &str) {
        let temporary = path.with_extension("toml.tmp");
        write_private(&temporary, contents);
        fs::rename(temporary, path).unwrap();
    }
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "wayexpand-store-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn reload_notifies_and_increments_generation() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        let receiver = store.subscribe();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'new'\n",
        );
        assert!(store.reload_if_changed().unwrap());
        assert_eq!(receiver.recv().unwrap(), 1);
        assert_eq!(store.snapshot().config.expansion[0].replacement, "new");
        let _ = fs::remove_file(path);
    }
    #[test]
    fn invalid_reload_keeps_last_valid_snapshot() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        write_private(&path, "replacement = [");
        assert!(store.reload_if_changed().is_err());
        let snapshot = store.snapshot();
        assert_eq!(snapshot.config.expansion[0].replacement, "old");
        assert_eq!(snapshot.generation, 0);
        assert!(
            !store.reload_if_changed().unwrap(),
            "unchanged invalid content must not be reparsed"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn atomic_renames_and_rapid_saves_install_the_latest_valid_snapshot() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        for value in 0..8 {
            atomic_write_private(
                &path,
                &format!("[[expansion]]\ntrigger = ':x'\nreplacement = 'value-{value}'\n"),
            );
            assert!(store.reload_if_changed().unwrap());
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.config.expansion[0].replacement, "value-7");
        assert_eq!(snapshot.generation, 8);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn subscriber_notifications_are_bounded_and_coalesced() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        let receiver = store.subscribe();
        for value in 0..64 {
            let mut config = (*store.snapshot().config).clone();
            config.expansion[0].replacement = format!("value-{value}");
            store.atomically_replace(config);
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.generation, 64);
        assert_eq!(receiver.recv().unwrap(), 1);
        assert!(receiver.try_recv().is_err());
        assert_eq!(snapshot.config.expansion[0].replacement, "value-63");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn snapshots_keep_configuration_and_generation_coherent_during_replacements() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        let writer = Arc::clone(&store);
        let join = thread::spawn(move || {
            for generation in 1..=500 {
                let mut config = (*writer.snapshot().config).clone();
                config.expansion[0].replacement = format!("generation-{generation}");
                writer.atomically_replace(config);
            }
        });
        while !join.is_finished() {
            let snapshot = store.snapshot();
            if snapshot.generation > 0 {
                assert_eq!(
                    snapshot.config.expansion[0].replacement,
                    format!("generation-{}", snapshot.generation)
                );
            }
            thread::yield_now();
        }
        join.join().unwrap();
        let snapshot = store.snapshot();
        assert_eq!(snapshot.generation, 500);
        assert_eq!(snapshot.config.expansion[0].replacement, "generation-500");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn seeded_snapshot_stress_keeps_configuration_and_generation_coherent() {
        const WRITES: u64 = 2_000;
        const READERS: usize = 4;
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'generation-0'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        let start = Arc::new(Barrier::new(READERS + 1));
        let mut readers = Vec::with_capacity(READERS);

        for reader_id in 0..READERS {
            let store = Arc::clone(&store);
            let finished = Arc::clone(&finished);
            let start = Arc::clone(&start);
            readers.push(thread::spawn(move || {
                let seed = std::env::var("WAYEXPAND_STRESS_SEED")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0x5741_5945_5850_414e)
                    ^ (reader_id as u64 + 1);
                let mut state = seed;
                start.wait();
                loop {
                    let snapshot = store.snapshot();
                    assert_eq!(
                        snapshot.config.expansion[0].replacement,
                        format!("generation-{}", snapshot.generation),
                        "reader {reader_id} observed a torn snapshot at generation {}",
                        snapshot.generation
                    );
                    if finished.load(Ordering::Acquire) {
                        break;
                    }
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    if state & 3 == 0 {
                        thread::yield_now();
                    } else {
                        for _ in 0..(state as u8) {
                            std::hint::spin_loop();
                        }
                    }
                }
            }));
        }

        let writer_store = Arc::clone(&store);
        let writer_start = Arc::clone(&start);
        let writer_finished = Arc::clone(&finished);
        let writer = thread::spawn(move || {
            let mut state = std::env::var("WAYEXPAND_STRESS_SEED")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0x5741_5945_5850_414e);
            writer_start.wait();
            for generation in 1..=WRITES {
                let mut config = (*writer_store.snapshot().config).clone();
                config.expansion[0].replacement = format!("generation-{generation}");
                writer_store.atomically_replace(config);
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                if state & 7 == 0 {
                    thread::yield_now();
                }
            }
            writer_finished.store(true, Ordering::Release);
        });

        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.generation, WRITES);
        assert_eq!(
            snapshot.config.expansion[0].replacement,
            format!("generation-{WRITES}")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn status_reports_reload_error_and_recovery() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        assert_eq!(
            store.status(),
            ConfigStoreStatus {
                state: "ok",
                error: None,
                generation: 0,
            }
        );

        write_private(&path, "replacement = [");
        assert!(store.reload_if_changed().is_err());
        let status = store.status();
        assert_eq!(status.state, "reload-error");
        assert_eq!(
            status.error.as_deref(),
            Some("invalid TOML at line 1, column 16")
        );
        assert_eq!(status.generation, 0);

        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'new'\n",
        );
        assert!(store.reload_if_changed().unwrap());
        assert_eq!(
            store.status(),
            ConfigStoreStatus {
                state: "ok",
                error: None,
                generation: 1,
            }
        );
        let _ = fs::remove_file(path);
    }
}
