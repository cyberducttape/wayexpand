use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use tracing::{error, info, warn};
use wayexpand_core::{
    limits::MAX_CONFIG_BYTES, Config, ConfigError, ExpansionEngine, FleetConfig, FleetError,
    OrganizationPolicy,
};

const MAX_CONSISTENCY_ATTEMPTS: usize = 3;
const INTEGRITY_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);
/// How often command workers that could not start are tried again.
const WORKER_RETRY_INTERVAL: Duration = Duration::from_secs(30);

pub struct ReloadableConfig {
    path: PathBuf,
    stamp: Option<FileStamp>,
    observed: Option<FileStamp>,
    last_integrity_check: Instant,
    watch_dirty: Arc<AtomicBool>,
    watch_check_after: Option<Instant>,
    _watcher: Option<RecommendedWatcher>,
    fleet: bool,
    fleet_signature: u64,
    policy: OrganizationPolicy,
    pub engine: ExpansionEngine,
    healthy: bool,
    /// The reload being read and built on a worker thread, if any.
    in_flight: Option<InFlightReload>,
    /// A change was seen while a build was in flight, so its result is
    /// already stale; build again once it lands.
    rebuild_after_in_flight: bool,
    /// Wakes the reactor when a background build finishes.
    waker: Option<crate::waker::Waker>,
    /// The daemon runs command snippets on worker threads. Kept here so a
    /// reload, or a retry after a failed start, starts them again.
    async_workers: bool,
    /// Commands are off by enforced organization policy, apart from whether
    /// workers are running.
    commands_restricted: bool,
    /// When to try again after command workers failed to start.
    worker_retry_at: Option<Instant>,
}

/// A configuration read, validated, and turned into a matcher off the
/// reactor. At the snippet limit that takes a few hundred milliseconds,
/// during which typing would otherwise stall.
struct InFlightReload {
    result: Receiver<BuildResult>,
    /// The file stamp seen when the build started.
    current: Option<FileStamp>,
}

struct BuiltConfig {
    engine: ExpansionEngine,
    expansions: usize,
    stamp: Option<FileStamp>,
    /// Fleet metadata as it existed before this load began. If a layer is
    /// edited while the build is running or before the reactor applies it,
    /// the next integrity check must still observe the change.
    fleet_signature: u64,
}

enum BuildFailure {
    /// Reading or validating the files failed.
    Load(anyhow::Error),
    /// The configuration could not become a matcher.
    Engine(anyhow::Error),
}

type BuildResult = Result<BuiltConfig, BuildFailure>;

fn build_config(path: &Path, fleet: bool, policy: &OrganizationPolicy) -> BuildResult {
    let fleet_signature = if fleet { standard_fleet_signature() } else { 0 };
    let (config, stamp) = load_for_mode(path, fleet, policy).map_err(BuildFailure::Load)?;
    let expansions = config.expansion.len();
    let engine = ExpansionEngine::new(config)
        .map_err(|error| BuildFailure::Engine(anyhow::Error::new(error)))?;
    Ok(BuiltConfig {
        engine,
        expansions,
        stamp,
        fleet_signature,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    length: u64,
    device: u64,
    inode: u64,
    mode: u32,
    change_time: i64,
    change_time_nsec: i64,
}

/// A metadata-only stamp for the primary config. This is sampled on the
/// keyboard reactor, so it must never read the file's contents. The actual
/// load and parse happen on the reload worker, which compares the stamp before
/// and after reading to reject concurrent edits.
fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let length = metadata.len();
    let device = metadata.dev();
    let inode = metadata.ino();
    let mode = metadata.mode();
    let change_time = metadata.ctime();
    let change_time_nsec = metadata.ctime_nsec();
    Some(FileStamp {
        modified,
        length,
        device,
        inode,
        mode,
        change_time,
        change_time_nsec,
    })
}

/// Bounded content read used only by reload workers, never by the input
/// reactor. It mirrors the config loader's regular-file and size limits and
/// rechecks the opened descriptor in case the path changes between metadata
/// lookup and open.
fn read_regular_config_file(path: &Path) -> Option<(fs::Metadata, Vec<u8>)> {
    let preliminary = fs::metadata(path).ok()?;
    if !preliminary.file_type().is_file() || preliminary.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let file = fs::File::from(descriptor);
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let mut contents = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut contents)
        .ok()?;
    (contents.len() <= MAX_CONFIG_BYTES).then_some((metadata, contents))
}

fn file_content_fingerprint(path: &Path) -> Option<u64> {
    let (_, contents) = read_regular_config_file(path)?;
    Some(stable_content_fingerprint(&contents))
}

fn stable_content_fingerprint(contents: &[u8]) -> u64 {
    // FNV-1a is deterministic across compiler versions and sensitive to byte
    // order, unlike a commutative chunk sum.
    contents.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn start_config_watcher(
    path: &Path,
    fleet: bool,
    dirty: Arc<AtomicBool>,
) -> Option<RecommendedWatcher> {
    let callback_dirty = Arc::clone(&dirty);
    let mut watcher = match notify::recommended_watcher(
        move |event: notify::Result<notify::Event>| {
            // Errors can mean an event was lost (for example, the kernel watch
            // queue overflowed), so they also request a secure rescan.
            let _ = event;
            callback_dirty.store(true, Ordering::Release);
        },
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            warn!(%error, "configuration filesystem watcher unavailable; using integrity polling");
            return None;
        }
    };

    let mut watched_config = false;
    let mut watched_paths = std::collections::HashSet::new();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        watched_paths.insert(parent.to_path_buf());
    } else {
        watched_paths.insert(PathBuf::from("."));
    }
    if let Ok(resolved) = fs::canonicalize(path) {
        if let Some(parent) = resolved.parent() {
            watched_paths.insert(parent.to_path_buf());
        }
    }
    for directory in watched_paths {
        match watcher.watch(&directory, RecursiveMode::NonRecursive) {
            Ok(()) => watched_config = true,
            Err(error) => {
                warn!(%error, path = %directory.display(), "could not watch configuration directory")
            }
        }
    }
    if !watched_config {
        warn!("configuration file watch unavailable; using integrity polling");
        return None;
    }

    if fleet {
        for directory in FleetConfig::standard_source_directories() {
            if !directory.is_dir() {
                continue;
            }
            if let Err(error) = watcher.watch(&directory, RecursiveMode::Recursive) {
                warn!(%error, path = %directory.display(), "could not watch fleet configuration directory; integrity polling remains enabled");
            }
        }
    }
    Some(watcher)
}

impl ReloadableConfig {
    pub fn load_with_fleet_and_policy(
        path: impl Into<PathBuf>,
        policy: OrganizationPolicy,
    ) -> Result<Self> {
        Self::load_mode_with_policy(path.into(), true, policy)
    }

    pub fn load_with_policy(path: impl Into<PathBuf>, policy: OrganizationPolicy) -> Result<Self> {
        Self::load_mode_with_policy(path.into(), false, policy)
    }

    fn load_mode_with_policy(
        path: PathBuf,
        fleet: bool,
        policy: OrganizationPolicy,
    ) -> Result<Self> {
        // Take the change-detection baseline before loading. If a fleet layer
        // changes during startup, the first integrity scan will see it rather
        // than accepting a post-load signature for an engine built from older
        // contents.
        let fleet_signature = if fleet { standard_fleet_signature() } else { 0 };
        let (config, stamp) = load_for_mode(&path, fleet, &policy)?;
        let engine = ExpansionEngine::new(config)
            .map_err(|error| anyhow::anyhow!("invalid configuration: {error}"))?;
        let watch_dirty = Arc::new(AtomicBool::new(false));
        let watcher = start_config_watcher(&path, fleet, Arc::clone(&watch_dirty));
        Ok(Self {
            path,
            stamp,
            observed: stamp,
            last_integrity_check: Instant::now(),
            watch_dirty,
            watch_check_after: None,
            _watcher: watcher,
            fleet,
            fleet_signature,
            policy,
            engine,
            healthy: true,
            in_flight: None,
            rebuild_after_in_flight: false,
            waker: None,
            async_workers: false,
            commands_restricted: false,
            worker_retry_at: None,
        })
    }

    /// Wake the reactor through `waker` when a background reload finishes.
    pub fn set_waker(&mut self, waker: crate::waker::Waker) {
        self.waker = Some(waker);
    }

    /// Run command snippets and hotkey actions on worker threads, now and
    /// after every reload. `restricted` is the enforced organization policy's
    /// decision to disable commands. Returns whether the
    /// workers started; if not, commands stay off and starting is retried.
    pub fn start_async_workers(&mut self, restricted: bool) -> bool {
        self.async_workers = true;
        self.commands_restricted = restricted;
        self.ensure_async_workers()
    }

    /// Start command workers if they should run and are not running, and
    /// set whether commands may run accordingly.
    fn ensure_async_workers(&mut self) -> bool {
        if !self.async_workers {
            // Not a daemon host: commands keep whatever state was inherited.
            return true;
        }
        let running = self.engine.enable_async_commands();
        self.worker_retry_at = (!running).then(|| Instant::now() + WORKER_RETRY_INTERVAL);
        self.engine
            .set_commands_disabled(self.commands_restricted || !running);
        running
    }

    fn retry_async_workers(&mut self) {
        if self
            .worker_retry_at
            .is_some_and(|retry_at| Instant::now() >= retry_at)
            && self.ensure_async_workers()
        {
            info!("command workers started; command-backed actions are enabled again");
        }
    }

    /// Parse first, then replace the live engine. Invalid edits leave the old
    /// configuration running and are reported to the operator.
    pub fn reload_if_changed(&mut self) {
        self.collect_reload();
        self.retry_async_workers();
        let now = Instant::now();
        if self.watch_dirty.swap(false, Ordering::AcqRel) && self.watch_check_after.is_none() {
            self.watch_check_after = Some(now + WATCH_DEBOUNCE);
        }
        let watch_due = self
            .watch_check_after
            .is_some_and(|deadline| now >= deadline);
        let integrity_due =
            now.duration_since(self.last_integrity_check) >= INTEGRITY_CHECK_INTERVAL;
        if !watch_due && !integrity_due {
            return;
        }
        if watch_due {
            self.watch_check_after = None;
        }
        if integrity_due {
            self.last_integrity_check = now;
        }

        let current = file_stamp(&self.path);
        let fleet_signature = self.fleet.then(standard_fleet_signature);
        let fleet_changed =
            fleet_signature.is_some_and(|signature| signature != self.fleet_signature);
        if current == self.observed && !fleet_changed {
            return;
        }
        self.observed = current;
        self.reload_current(current);
    }

    pub fn reload_now(&mut self) {
        // Do not collect an already-finished build before recording this
        // request. The worker may finish between the caller's edit and this
        // method, which otherwise makes a second reload depend on scheduling:
        // the first result can be applied and the follow-up request can be
        // lost at the boundary. Queue the request explicitly and let
        // `collect_reload` start the follow-up after the current result is
        // applied.
        if self.in_flight.is_some() {
            self.rebuild_after_in_flight = true;
            let current = file_stamp(&self.path);
            self.last_integrity_check = Instant::now();
            self.watch_check_after = None;
            self.watch_dirty.store(false, Ordering::Release);
            self.observed = current;
            return;
        }
        self.collect_reload();
        let current = file_stamp(&self.path);
        self.last_integrity_check = Instant::now();
        self.watch_check_after = None;
        self.watch_dirty.store(false, Ordering::Release);
        self.observed = current;
        self.reload_current(current);
    }

    /// Start building the configuration on disk off the reactor; its result
    /// replaces the engine on a later turn (see [`Self::collect_reload`]).
    fn reload_current(&mut self, current: Option<FileStamp>) {
        if self.in_flight.is_some() {
            self.rebuild_after_in_flight = true;
            return;
        }
        let (sender, result) = mpsc::sync_channel(1);
        let path = self.path.clone();
        let fleet = self.fleet;
        let policy = self.policy.clone();
        let waker = self.waker.clone();
        let spawned = thread::Builder::new()
            .name("wayexpand-reload".into())
            .spawn(move || {
                // A dropped receiver (daemon stopping) just discards it.
                let _ = sender.send(build_config(&path, fleet, &policy));
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        match spawned {
            Ok(_) => self.in_flight = Some(InFlightReload { result, current }),
            Err(error) => {
                // Without a worker, reloading inline is slower but correct.
                warn!(%error, "could not start configuration reload worker; reloading inline");
                let built = build_config(&self.path, self.fleet, &self.policy);
                self.apply_reload(built, current);
            }
        }
    }

    /// Apply a finished background reload, without waiting for one that is
    /// still being built.
    fn collect_reload(&mut self) {
        let Some(in_flight) = self.in_flight.as_ref() else {
            return;
        };
        let built = match in_flight.result.try_recv() {
            Ok(built) => built,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(BuildFailure::Load(anyhow::anyhow!(
                "configuration reload worker ended without a result"
            ))),
        };
        let current = in_flight.current;
        self.in_flight = None;
        self.apply_reload(built, current);
        if std::mem::take(&mut self.rebuild_after_in_flight) {
            let current = file_stamp(&self.path);
            self.observed = current;
            self.reload_current(current);
        }
    }

    /// Wait for a background reload and apply it.
    #[cfg(test)]
    pub fn finish_pending_reload(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.in_flight.is_some() || self.rebuild_after_in_flight {
            assert!(
                Instant::now() < deadline,
                "configuration reload never finished"
            );
            self.collect_reload();
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Swap in a built engine, carrying the live runtime state over, or
    /// keep the running configuration and report why the new one was refused.
    fn apply_reload(&mut self, built: BuildResult, current: Option<FileStamp>) {
        match built {
            Ok(BuiltConfig {
                mut engine,
                expansions,
                stamp: stable_stamp,
                fleet_signature,
            }) => {
                // Pause, password-field, composition and window state,
                // command restrictions and host hooks all carry over, as of
                // now rather than when the build started; losing any of them
                // would, for example, resume matching inside a password field.
                engine.inherit_runtime_state(&self.engine);
                // A host that enabled workers on the engine directly wants
                // them after the reload too.
                self.async_workers |= self.engine.async_commands_enabled();
                // Stop the old command workers before starting new ones, so
                // a reload never holds two sets of threads against the
                // unit's task limit.
                drop(std::mem::replace(&mut self.engine, engine));
                if !self.ensure_async_workers() {
                    warn!(
                        "command workers could not restart after configuration reload; command-backed actions are disabled until they start (retried every 30 s)"
                    );
                }
                self.stamp = stable_stamp;
                self.observed = stable_stamp;
                self.last_integrity_check = Instant::now();
                self.fleet_signature = fleet_signature;
                self.healthy = true;
                info!(expansions, "configuration reloaded");
            }
            Err(BuildFailure::Engine(error)) => {
                self.healthy = false;
                error!(
                    reason = %safe_reload_error(&error),
                    "configuration reload rejected; keeping previous configuration"
                )
            }
            Err(BuildFailure::Load(error)) => {
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
}

fn safe_reload_error(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<ConfigError>() {
        return error.safe_summary();
    }
    // Previously wrapped in an untyped message, so a fleet conflict was
    // logged as "configuration could not be read consistently".
    if let Some(error) = error.downcast_ref::<FleetError>() {
        return format!("fleet configuration invalid: {}", error.safe_summary());
    }
    "configuration could not be read consistently".into()
}

fn load_consistent(path: &Path) -> Result<(Config, Option<FileStamp>)> {
    for _attempt in 0..MAX_CONSISTENCY_ATTEMPTS {
        let before = file_stamp(path);
        let content_before = file_content_fingerprint(path);
        let config = Config::load(path)?;
        let after = file_stamp(path);
        let content_after = file_content_fingerprint(path);
        if before == after && content_before == content_after {
            return Ok((config, after));
        }
    }
    anyhow::bail!(
        "configuration changed while being read after {MAX_CONSISTENCY_ATTEMPTS} attempts"
    )
}

fn load_for_mode(
    path: &Path,
    fleet: bool,
    policy: &OrganizationPolicy,
) -> Result<(Config, Option<FileStamp>)> {
    // Layer files in snippets.d are validated by the fleet loader, which is
    // the only daemon path that reads them (and follows symlinks as it does).
    // A layer can change while that merge is in progress, so take a signature
    // before and after the full load and retry rather than briefly promoting a
    // mixed snapshot of the library.
    for attempt in 0..MAX_CONSISTENCY_ATTEMPTS {
        let (mut base, stamp) = load_consistent(path)?;
        if fleet {
            let before = standard_fleet_content_signature();
            // Kept typed so `safe_reload_error` can report what went wrong.
            let merged = FleetConfig::load_standard_with_base_and_policy(base, policy)
                .map_err(anyhow::Error::new)?;
            let after = standard_fleet_content_signature();
            if before != after {
                if attempt + 1 == MAX_CONSISTENCY_ATTEMPTS {
                    anyhow::bail!(
                        "fleet configuration changed while being read after {MAX_CONSISTENCY_ATTEMPTS} attempts"
                    );
                }
                continue;
            }
            for violation in &merged.policy_violations {
                super::policy::log_violation(policy, violation);
            }
            base = merged.config;
        }

        base.apply_administrator_policy(policy)
            .map_err(anyhow::Error::new)?;

        return Ok((base, stamp));
    }
    unreachable!("fleet consistency loop returns or fails on every attempt")
}

fn standard_fleet_signature() -> u64 {
    match FleetConfig::standard_source_files() {
        Ok(paths) => fleet_signature_for(paths),
        Err(error) => {
            let mut hasher = DefaultHasher::new();
            error.to_string().hash(&mut hasher);
            hasher.finish()
        }
    }
}

fn fleet_signature_for(paths: impl IntoIterator<Item = PathBuf>) -> u64 {
    let mut hasher = DefaultHasher::new();
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort();
    for path in paths {
        path.hash(&mut hasher);
        // This probe runs on the keyboard reactor when a filesystem event
        // arrives and during periodic integrity checks. Hash metadata only:
        // reading every layer here can synchronously pull gigabytes through
        // the input path for a large library or slow home filesystem. The
        // background loader still reads and validates the complete contents
        // before it can promote a new engine.
        match fs::metadata(&path) {
            Ok(metadata) => {
                b"present".hash(&mut hasher);
                metadata.file_type().is_file().hash(&mut hasher);
                metadata.len().hash(&mut hasher);
                metadata.modified().ok().hash(&mut hasher);
                metadata.ino().hash(&mut hasher);
                metadata.dev().hash(&mut hasher);
                metadata.ctime().hash(&mut hasher);
                metadata.ctime_nsec().hash(&mut hasher);
            }
            Err(error) => {
                b"unreadable".hash(&mut hasher);
                error.kind().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// Exact content signature for the background fleet load consistency check.
/// Keep content I/O off the reactor; `fleet_signature_for` above is the fast
/// metadata-only change probe used there.
fn standard_fleet_content_signature() -> u64 {
    match FleetConfig::standard_source_files() {
        Ok(paths) => {
            let mut hasher = DefaultHasher::new();
            let mut paths: Vec<_> = paths.into_iter().collect();
            paths.sort();
            for path in paths {
                path.hash(&mut hasher);
                match read_regular_config_file(&path) {
                    Some((_, contents)) => {
                        b"contents".hash(&mut hasher);
                        stable_content_fingerprint(&contents).hash(&mut hasher);
                    }
                    None => {
                        b"unavailable".hash(&mut hasher);
                        if let Ok(metadata) = fs::metadata(&path) {
                            metadata.len().hash(&mut hasher);
                            metadata.ino().hash(&mut hasher);
                            metadata.ctime().hash(&mut hasher);
                            metadata.ctime_nsec().hash(&mut hasher);
                        }
                    }
                }
            }
            hasher.finish()
        }
        Err(error) => {
            let mut hasher = DefaultHasher::new();
            error.to_string().hash(&mut hasher);
            hasher.finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };
    use wayexpand_core::InputEvent;

    fn temporary_config() -> PathBuf {
        static NEXT_TEMP_CONFIG: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos()
            ^ u128::from(NEXT_TEMP_CONFIG.fetch_add(1, Ordering::Relaxed));
        std::env::temp_dir().join(format!("wayexpand-reload-test-{nonce}.toml"))
    }

    fn isolated_temporary_config() -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-reload-test-dir-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("expansions.toml");
        (directory, path)
    }

    fn config_text(replacement: &str) -> String {
        format!("[[expansion]]\ntrigger = \":x\"\nreplacement = {replacement:?}\n")
    }

    #[test]
    fn fleet_signature_tracks_same_size_nested_pack_rewrites() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-fleet-signature-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = root.join("my-pack").join("foo.toml");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&nested, "before").unwrap();
        assert_eq!(fs::metadata(&nested).unwrap().len(), 6);

        let before = fleet_signature_for([nested.clone()]);
        // Config editors save by writing a sibling and atomically renaming it
        // into place. Reproduce that replacement rather than relying on the
        // filesystem to expose sub-second ctime changes for an in-place write
        // (some Launchpad builders use filesystems with coarser metadata).
        let replacement = nested.with_extension("toml.new");
        fs::write(&replacement, "after!").unwrap();
        fs::rename(&replacement, &nested).unwrap();
        assert_eq!(fs::metadata(&nested).unwrap().len(), 6);
        let after = fleet_signature_for([nested.clone()]);

        assert_ne!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_content_fingerprint_tracks_same_size_rewrites() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-config-fingerprint-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("expansions.toml");
        fs::write(&config, "before").unwrap();
        let before = file_content_fingerprint(&config);
        fs::write(&config, "after!").unwrap();
        assert_eq!(fs::metadata(&config).unwrap().len(), 6);
        let after = file_content_fingerprint(&config);

        assert_ne!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fleet_signature_never_blocks_on_a_fifo_named_like_a_layer() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-fleet-fifo-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let fifo = root.join("hang.toml");
        let link = root.join("link.toml");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        std::os::unix::fs::symlink(&fifo, &link).unwrap();

        let (done, finished) = std::sync::mpsc::channel();
        let paths = vec![fifo.clone(), link.clone()];
        thread::spawn(move || {
            let _ = done.send(fleet_signature_for(paths));
        });
        // Without a writer, a blocking open of the FIFO never returns.
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "fleet integrity probe blocked on a FIFO"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Writes a fixture with an explicit private mode.    /// Writes a fixture with an explicit private mode. Relying on the
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
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [\"app_id_exact:org.kde.kate\"]\n",
        );
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        config.engine.set_current_window(Some(WindowContext {
            app_id: Some("org.kde.kate".into()),
            title: None,
            instance_id: None,
        }));

        // Any reload -- including one an unrelated GUI edit would trigger --
        // must not forget the window the user is actually still in.
        write_config(
            &path,
            "[[expansion]]\ntrigger = \":x\"\nreplacement = \"z\"\napp_filter = [\"app_id_exact:org.kde.kate\"]\n",
        );
        config.reload_now();
        config.finish_pending_reload();
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
    fn non_fleet_load_applies_administrator_absolute_command_policy() {
        let path = temporary_config();
        write_config(
            &path,
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["ok"]
            "#,
        );
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };

        let error = match ReloadableConfig::load_with_policy(&path, policy) {
            Ok(_) => panic!("non-fleet load must enforce the administrator policy"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("absolute path"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn non_fleet_audit_policy_allows_relative_command_programs() {
        let path = temporary_config();
        write_config(
            &path,
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["ok"]
            "#,
        );
        let policy = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };

        let config = ReloadableConfig::load_with_policy(&path, policy).unwrap();
        assert!(config.healthy());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn valid_reload_replaces_active_engine() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, &config_text("new replacement"));
        config.reload_now();
        config.finish_pending_reload();
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
    fn applying_reload_keeps_the_worker_captured_fleet_signature() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        let engine = ExpansionEngine::new(Config::parse(&config_text("new")).unwrap()).unwrap();
        let stamp = file_stamp(&path);
        let captured_signature = 0x51_7a_9e;

        config.apply_reload(
            Ok(BuiltConfig {
                engine,
                expansions: 1,
                stamp,
                fleet_signature: captured_signature,
            }),
            stamp,
        );

        assert_eq!(config.fleet_signature, captured_signature);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_reload_keeps_previous_engine() {
        let path = temporary_config();
        write_config(&path, &config_text("stable"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, "[[expansion]]\ntrigger = ");
        config.reload_now();
        config.finish_pending_reload();
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
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        fs::remove_file(&path).unwrap();
        config.reload_now();
        config.finish_pending_reload();
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "before outage");

        write_config(&path, &config_text("after restore"));
        config.reload_now();
        config.finish_pending_reload();
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
    fn command_workers_restart_after_every_reload_without_overlapping() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        assert!(config.start_async_workers(false));
        assert!(config.engine.async_commands_enabled());
        assert!(!config.engine.commands_disabled());
        for replacement in ["one", "two", "three"] {
            write_config(&path, &config_text(replacement));
            config.reload_now();
            config.finish_pending_reload();
            assert!(config.engine.async_commands_enabled(), "{replacement}");
            assert!(!config.engine.commands_disabled(), "{replacement}");
        }
        let _ = fs::remove_file(path);
    }

    #[test]
    fn commands_disabled_by_a_failed_worker_start_come_back_on_retry() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        // As after a failed start: commands off and a retry scheduled.
        config.async_workers = true;
        config.engine.set_commands_disabled(true);
        config.worker_retry_at = Some(Instant::now());
        config.retry_async_workers();
        assert!(config.engine.async_commands_enabled());
        assert!(!config.engine.commands_disabled());
        assert_eq!(config.worker_retry_at, None);

        // A reload after a failure also starts them, instead of carrying
        // the disabled state forward as it used to.
        config.engine.set_commands_disabled(true);
        write_config(&path, &config_text("new"));
        config.reload_now();
        config.finish_pending_reload();
        assert!(config.engine.async_commands_enabled());
        assert!(!config.engine.commands_disabled());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn an_enforced_command_policy_survives_reloads() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        assert!(config.start_async_workers(true));
        assert!(config.engine.commands_disabled());
        write_config(&path, &config_text("new"));
        config.reload_now();
        config.finish_pending_reload();
        assert!(config.engine.commands_disabled());
        assert!(config.engine.async_commands_enabled());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn a_reload_is_built_off_the_caller_and_swapped_in_later() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        let waker = crate::waker::Waker::new().unwrap();
        config.set_waker(waker.clone());
        write_config(&path, &config_text("new"));

        config.reload_now();
        // The build runs elsewhere; until it is collected, typing keeps
        // using the running engine instead of waiting for it.
        assert!(config.in_flight.is_some());
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "old");

        config.finish_pending_reload();
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "new");
        assert!(config.healthy());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn state_changed_while_a_reload_builds_carries_into_the_new_engine() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, &config_text("new"));

        config.reload_now();
        // Paused after the build started: the swap must not resume matching.
        config.engine.process(InputEvent::PauseChanged(true));
        config.finish_pending_reload();
        assert!(config
            .engine
            .process(InputEvent::Text(":x".into()))
            .is_empty());
        config.engine.process(InputEvent::PauseChanged(false));
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn a_change_during_a_reload_is_built_after_it() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, &config_text("first"));
        config.reload_now();
        write_config(&path, &config_text("second"));
        config.reload_now();
        config.finish_pending_reload();
        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "second");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn in_place_same_size_edit_is_detected() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, &config_text("new"));
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            config.reload_if_changed();
            let result = config
                .engine
                .process(InputEvent::Text(":x".into()))
                .pop()
                .unwrap();
            if result.insert == "new" {
                break result;
            }
            assert!(
                Instant::now() < deadline,
                "filesystem change was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(result.insert, "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn atomic_replacement_is_detected_by_watching_the_parent_directory() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        let replacement = path.with_extension("replacement");
        write_config(&replacement, &config_text("new"));
        fs::rename(&replacement, &path).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            config.reload_if_changed();
            let result = config
                .engine
                .process(InputEvent::Text(":x".into()))
                .pop()
                .unwrap();
            if result.insert == "new" {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "atomic replacement was not observed"
            );
            thread::sleep(Duration::from_millis(10));
        }
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
    fn unchanged_configuration_does_not_scan_until_integrity_deadline() {
        let (directory, path) = isolated_temporary_config();
        write_config(&path, &config_text("stable metadata"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();

        // The watcher is asynchronous and can keep delivering/coalescing its
        // initial directory event while other tests are creating temporary
        // configs. This test covers the integrity-polling path, not watcher
        // startup, so establish the steady-state inputs directly.
        config.watch_check_after = None;
        config.watch_dirty.store(false, Ordering::Release);

        let stamp = config.observed.unwrap();
        config.last_integrity_check = Instant::now();
        config.reload_if_changed();
        assert_eq!(config.observed, Some(stamp));
        assert!(config.watch_check_after.is_none());
        let _ = fs::remove_file(&path);
        let filename = path.file_name().unwrap().to_string_lossy();
        let _ = fs::remove_file(directory.join(format!(".{filename}.wayexpand.lock")));
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn integrity_fallback_detects_changes_when_no_watch_event_arrives() {
        let path = temporary_config();
        write_config(&path, &config_text("old"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        write_config(&path, &config_text("new"));
        config.last_integrity_check = Instant::now() - INTEGRITY_CHECK_INTERVAL;
        config.watch_dirty.store(false, Ordering::Release);
        config.reload_if_changed();
        config.finish_pending_reload();

        let result = config
            .engine
            .process(InputEvent::Text(":x".into()))
            .pop()
            .unwrap();
        assert_eq!(result.insert, "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_while_a_command_runs_drops_its_result_and_keeps_typing_working() {
        let slow = "[[expansion]]\ntrigger = \":slow\"\nreplacement = \"\"\n\
                    [expansion.command]\nprogram = \"/bin/sh\"\n\
                    args = [\"-c\", \"sleep 0.3; printf command-output\"]\ntimeout_ms = 2000\n";
        let path = temporary_config();
        write_config(&path, &format!("{slow}{}", config_text("before")));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
        assert!(config.engine.enable_async_commands());

        assert!(config
            .engine
            .process(InputEvent::Text(":slow".into()))
            .is_empty());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while config.engine.command_metrics().command_in_flight == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "command did not start"
            );
            std::thread::sleep(Duration::from_millis(2));
        }

        // Reload a changed snippet while the command runs, and keep typing.
        write_config(&path, &format!("{slow}{}", config_text("after")));
        config.reload_now();
        config.finish_pending_reload();
        let typed = config.engine.process(InputEvent::Text(":x".into()));
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].insert, "after");

        // The old engine's command must never deliver into the new engine.
        let quiet = std::time::Instant::now() + Duration::from_millis(600);
        while std::time::Instant::now() < quiet {
            assert!(config.engine.drain_completed_commands().is_empty());
            std::thread::sleep(Duration::from_millis(20));
        }

        // The reloaded engine still runs commands.
        assert!(config
            .engine
            .process(InputEvent::Text(":slow".into()))
            .is_empty());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let completed = loop {
            if let Some(result) = config.engine.drain_completed_commands().pop() {
                break result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reloaded command did not complete"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(completed.insert, "command-output");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_preserves_runtime_safety_state() {
        // CRITICAL P0 SECURITY TEST: Verify that config reloads do NOT
        // lose sensitive_focus, user_paused, or composition state.
        //
        // A fresh ExpansionEngine defaults both to false, meaning:
        // - If sensitive_focus was true (password field), a reload would
        //   resume matching in that field until the next compositor event.
        // - If user_paused was true, a reload would appear to resume matching
        //   despite the control API still reporting pause.
        //
        // Composition state is equally critical: matching during a dead-key,
        // Compose, or IME preedit can corrupt the user's in-progress text.
        let path = temporary_config();
        write_config(&path, &config_text("initial"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();

        // Simulate entering a sensitive field and pausing the user.
        config.engine.set_sensitive_focus(true);
        config.engine.set_user_paused(true);
        config.engine.set_composition_active(true);
        assert!(config.engine.is_sensitive_focus());
        assert!(config.engine.is_user_paused());
        assert!(config.engine.is_composition_active());

        // Trigger a reload (e.g., from a GUI save).
        write_config(&path, &config_text("reloaded"));
        config.reload_now();
        config.finish_pending_reload();
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
        assert!(
            config.engine.is_composition_active(),
            "composition_active lost on reload - matching resumed during preedit!"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn composition_state_survives_reload_until_preedit_ends() {
        let path = temporary_config();
        write_config(&path, &config_text("initial"));
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();

        config
            .engine
            .process(InputEvent::CompositionChanged { active: true });
        write_config(&path, &config_text("reloaded"));
        config.reload_now();
        config.finish_pending_reload();

        assert!(config.engine.is_composition_active());
        assert!(
            config
                .engine
                .process(InputEvent::Text(":x".into()))
                .is_empty(),
            "reload must not resume matching during active composition"
        );

        config
            .engine
            .process(InputEvent::CompositionChanged { active: false });
        let results = config.engine.process(InputEvent::Text(":x".into()));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].insert, "reloaded");
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
