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
    config: RwLock<Arc<Config>>,
    generation: Mutex<u64>,
    /// Metadata observed on the most recent poll, whether or not it parsed.
    /// Keeping this separate from `applied_stamp` prevents unchanged invalid
    /// content from being reparsed on every fallback-poll cycle.
    observed_stamp: Mutex<Option<FileStamp>>,
    reload_error: Mutex<Option<String>>,
    subscribers: Mutex<Vec<mpsc::SyncSender<u64>>>,
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
            config: RwLock::new(Arc::new(loaded.config)),
            generation: Mutex::new(0),
            reload_error: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
        }))
    }
    pub fn config(&self) -> Arc<Config> {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub fn generation(&self) -> u64 {
        *self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
            generation: self.generation(),
        }
    }
    pub fn subscribe(&self) -> mpsc::Receiver<u64> {
        // A subscriber only needs to learn that a newer snapshot exists; the
        // current config and generation are read from the store. Capacity one
        // prevents a stalled consumer from retaining an unbounded history.
        let (sender, receiver) = mpsc::sync_channel(1);
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sender);
        receiver
    }
    pub fn atomically_replace(&self, config: Config) -> u64 {
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(config);
        let mut generation = self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *generation = generation.wrapping_add(1);
        let value = *generation;
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|subscriber| match subscriber.try_send(value) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            });
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

            // Read and validate a second descriptor snapshot. Comparing the
            // exact source revision ensures the config being installed is the
            // same content represented by the revision we just observed, even
            // when an editor atomically renames the file during the first
            // read.
            let verified = match Config::load_versioned(&self.path) {
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
            if before == after && loaded.revision == verified.revision {
                self.atomically_replace(verified.config);
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
        assert_eq!(store.config().expansion[0].replacement, "new");
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
        assert_eq!(store.config().expansion[0].replacement, "old");
        assert_eq!(store.generation(), 0);
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
        assert_eq!(store.config().expansion[0].replacement, "value-7");
        assert_eq!(store.generation(), 8);
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
            let mut config = (*store.config()).clone();
            config.expansion[0].replacement = format!("value-{value}");
            store.atomically_replace(config);
        }
        assert_eq!(store.generation(), 64);
        assert_eq!(receiver.recv().unwrap(), 1);
        assert!(receiver.try_recv().is_err());
        assert_eq!(store.config().expansion[0].replacement, "value-63");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn poisoned_generation_lock_is_recovered_without_panicking() {
        let path = path();
        write_private(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        );
        let store = ConfigStore::load(&path).unwrap();
        let poisoned = Arc::clone(&store);
        let join = thread::spawn(move || {
            let _guard = poisoned.generation.lock().unwrap();
            panic!("test poison");
        })
        .join();
        assert!(join.is_err());
        assert_eq!(store.generation(), 0);
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
