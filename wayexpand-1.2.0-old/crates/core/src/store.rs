use crate::{Config, ConfigError};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, PoisonError, RwLock},
    time::SystemTime,
};

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
    subscribers: Mutex<Vec<mpsc::Sender<u64>>>,
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
        let config = Config::load(&path)?;
        Ok(Arc::new(Self {
            observed_stamp: Mutex::new(metadata_stamp(&path)),
            path,
            config: RwLock::new(Arc::new(config)),
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
        let (sender, receiver) = mpsc::channel();
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
            .retain(|subscriber| subscriber.send(value).is_ok());
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
        // Record the observation even when parsing fails. The active config
        // remains untouched, but an unchanged invalid file is not a new
        // reload attempt on the next poll.
        *observed_stamp = current;
        let config = match Config::load(&self.path) {
            Ok(config) => config,
            Err(error) => {
                *self
                    .reload_error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error.safe_summary());
                return Err(error);
            }
        };
        self.atomically_replace(config);
        *self
            .reload_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        Ok(true)
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
        assert_eq!(status.error.as_deref(), Some("invalid TOML"));
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
