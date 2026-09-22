use crate::{Config, ConfigError};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, RwLock},
};

/// Shared validated configuration snapshot with generation notifications.
pub struct ConfigStore {
    path: PathBuf,
    config: RwLock<Arc<Config>>,
    generation: Mutex<u64>,
    stamp: Mutex<Option<u64>>,
    subscribers: Mutex<Vec<mpsc::Sender<u64>>>,
}

impl ConfigStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<Arc<Self>, ConfigError> {
        let path = path.into();
        let config = Config::load(&path)?;
        Ok(Arc::new(Self {
            stamp: Mutex::new(fingerprint(&path)),
            path,
            config: RwLock::new(Arc::new(config)),
            generation: Mutex::new(0),
            subscribers: Mutex::new(Vec::new()),
        }))
    }
    pub fn config(&self) -> Arc<Config> {
        self.config
            .read()
            .expect("configuration lock poisoned")
            .clone()
    }
    pub fn generation(&self) -> u64 {
        *self.generation.lock().expect("generation lock poisoned")
    }
    pub fn subscribe(&self) -> mpsc::Receiver<u64> {
        let (sender, receiver) = mpsc::channel();
        self.subscribers
            .lock()
            .expect("subscriber lock poisoned")
            .push(sender);
        receiver
    }
    pub fn atomically_replace(&self, config: Config) -> u64 {
        *self.config.write().expect("configuration lock poisoned") = Arc::new(config);
        let mut generation = self.generation.lock().expect("generation lock poisoned");
        *generation = generation.wrapping_add(1);
        let value = *generation;
        self.subscribers
            .lock()
            .expect("subscriber lock poisoned")
            .retain(|subscriber| subscriber.send(value).is_ok());
        value
    }
    /// Invalid edits leave the last valid snapshot active.
    pub fn reload_if_changed(&self) -> Result<bool, ConfigError> {
        let current = fingerprint(&self.path);
        let mut stamp = self.stamp.lock().expect("stamp lock poisoned");
        if current == *stamp {
            return Ok(false);
        }
        let config = Config::load(&self.path)?;
        *stamp = current;
        self.atomically_replace(config);
        Ok(true)
    }
}

fn fingerprint(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
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
        fs::write(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        let receiver = store.subscribe();
        fs::write(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'new'\n",
        )
        .unwrap();
        assert!(store.reload_if_changed().unwrap());
        assert_eq!(receiver.recv().unwrap(), 1);
        assert_eq!(store.config().expansion[0].replacement, "new");
        let _ = fs::remove_file(path);
    }
    #[test]
    fn invalid_reload_keeps_last_valid_snapshot() {
        let path = path();
        fs::write(
            &path,
            "[[expansion]]\ntrigger = ':x'\nreplacement = 'old'\n",
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        fs::write(&path, "not valid toml").unwrap();
        assert!(store.reload_if_changed().is_err());
        assert_eq!(store.config().expansion[0].replacement, "old");
        assert_eq!(store.generation(), 0);
        let _ = fs::remove_file(path);
    }
}
