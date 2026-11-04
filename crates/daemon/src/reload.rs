use anyhow::Result;
mod files;
#[cfg(test)]
use files::{event_requests_rescan, WatchFilter};
use files::{
    file_content_fingerprint, file_stamp, read_regular_config_file, stable_content_fingerprint,
    start_config_watcher, FileStamp,
};
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
    /// Old engines with live command workers are dropped by this reaper so
    /// their cooperative shutdown joins never run on the input reactor.
    retired_engine_sender: mpsc::Sender<RetiredEngine>,
    /// Completion of the currently retiring runtime. New workers wait until
    /// the old ones have exited, preserving the per-daemon worker limit.
    retiring_runtime: Option<Receiver<()>>,
}

/// A configuration read, validated, and turned into a matcher off the
/// reactor. At the snippet limit that takes a few hundred milliseconds,
/// during which typing would otherwise stall.
struct InFlightReload {
    result: Receiver<BuildResult>,
    path: PathBuf,
    /// The file stamp seen when the build started.
    current: Option<FileStamp>,
    /// Metadata signature of fleet layers when this request was queued.
    fleet_signature: u64,
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

struct RetiredEngine {
    engine: ExpansionEngine,
    finished: mpsc::SyncSender<()>,
    waker: Option<crate::waker::Waker>,
}

fn build_config(path: &Path, fleet: bool, policy: &OrganizationPolicy) -> BuildResult {
    for attempt in 0..MAX_CONSISTENCY_ATTEMPTS {
        let fleet_before = if fleet { standard_fleet_signature() } else { 0 };
        let loaded = load_for_mode(path, fleet, policy);
        let fleet_after = if fleet { standard_fleet_signature() } else { 0 };
        if fleet_before != fleet_after {
            if attempt + 1 == MAX_CONSISTENCY_ATTEMPTS {
                return Err(BuildFailure::Load(anyhow::anyhow!(
                    "fleet configuration changed while being built after {MAX_CONSISTENCY_ATTEMPTS} attempts"
                )));
            }
            continue;
        }
        let (config, stamp) = loaded.map_err(BuildFailure::Load)?;
        let expansions = config.expansion.len();
        let snapshot = config
            .validate_snapshot()
            .map_err(|error| BuildFailure::Engine(anyhow::Error::new(error)))?;
        let engine = ExpansionEngine::from_validated(Arc::new(snapshot));
        return Ok(BuiltConfig {
            engine,
            expansions,
            stamp,
            fleet_signature: fleet_after,
        });
    }
    unreachable!("fleet consistency loop returns or fails on every attempt")
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
        let snapshot = config
            .validate_snapshot()
            .map_err(|error| anyhow::anyhow!("invalid configuration: {error}"))?;
        let engine = ExpansionEngine::from_validated(Arc::new(snapshot));
        let (retired_engine_sender, retired_engine_receiver) = mpsc::channel::<RetiredEngine>();
        thread::Builder::new()
            .name("wayexpand-engine-reaper".into())
            .spawn(move || {
                while let Ok(retired) = retired_engine_receiver.recv() {
                    drop(retired.engine);
                    let _ = retired.finished.send(());
                    if let Some(waker) = retired.waker {
                        waker.wake();
                    }
                }
            })?;
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
            retired_engine_sender,
            retiring_runtime: None,
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
        if let Some(retiring) = &self.retiring_runtime {
            match retiring.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => self.retiring_runtime = None,
                Err(TryRecvError::Empty) => {
                    self.engine
                        .set_commands_disabled(self.commands_restricted || self.async_workers);
                    self.worker_retry_at = None;
                    return false;
                }
            }
        }
        let running = self.engine.enable_async_commands();
        self.worker_retry_at = (!running).then(|| Instant::now() + WORKER_RETRY_INTERVAL);
        self.engine
            .set_commands_disabled(self.commands_restricted || !running);
        running
    }

    fn retry_async_workers(&mut self) {
        if self.retiring_runtime.is_some() {
            if self.ensure_async_workers() {
                info!("command workers restarted after old runtime shutdown");
            }
            return;
        }
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
        // A watcher event is itself evidence that configuration contents may
        // have changed. Some filesystems expose coarse timestamps and can
        // leave the metadata signature unchanged for a same-size in-place
        // write. Reload on the debounced event; parsing and validation happen
        // on the background worker, not on the input reactor.
        if current == self.observed && !fleet_changed && !watch_due {
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

    /// Select a different primary library. The old engine remains installed
    /// while the new one is built; callers should pause matching until the
    /// selected path is healthy.
    pub fn switch_path(&mut self, path: PathBuf) {
        if self.path == path && self.healthy {
            self.reload_now();
            return;
        }
        if self.path == path
            && self
                .in_flight
                .as_ref()
                .is_some_and(|load| load.path == path)
        {
            return;
        }
        self.path = path;
        self.healthy = false;
        self.stamp = None;
        self.observed = file_stamp(&self.path);
        self.watch_dirty.store(false, Ordering::Release);
        self.watch_check_after = None;
        self.last_integrity_check = Instant::now();
        self._watcher = start_config_watcher(&self.path, self.fleet, Arc::clone(&self.watch_dirty));
        if self.in_flight.is_some() {
            self.rebuild_after_in_flight = true;
        } else {
            self.reload_current(self.observed);
        }
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
        let in_flight_path = path.clone();
        let fleet = self.fleet;
        let fleet_signature = if fleet { standard_fleet_signature() } else { 0 };
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
            Ok(_) => {
                self.in_flight = Some(InFlightReload {
                    result,
                    current,
                    path: in_flight_path,
                    fleet_signature,
                })
            }
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
        let requested_fleet_signature = in_flight.fleet_signature;
        let built_path = in_flight.path.clone();
        self.in_flight = None;
        if built_path == self.path {
            let current_stamp = file_stamp(&self.path);
            let current_fleet_signature = if self.fleet {
                standard_fleet_signature()
            } else {
                0
            };
            let built_snapshot_is_current = match &built {
                Ok(built) => {
                    built.stamp == current_stamp && built.fleet_signature == current_fleet_signature
                }
                Err(_) => {
                    current == current_stamp && requested_fleet_signature == current_fleet_signature
                }
            };
            if built_snapshot_is_current {
                self.apply_reload(built, current);
            } else {
                // A validated build can still become stale before the reactor
                // promotes it. Keep the active engine and build the current
                // on-disk generation instead of briefly installing old text.
                self.observed = current_stamp;
                self.rebuild_after_in_flight = true;
            }
        }
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
                // The old runtime's Drop joins its command and hotkey workers.
                // Hand it to a reaper so a blocked cancellation cannot stall
                // this input-reactor turn. New workers remain disabled until
                // the reaper confirms the old set has exited.
                let old_engine = std::mem::replace(&mut self.engine, engine);
                let old_runtime_enabled = old_engine.async_commands_enabled();
                self.async_workers |= old_runtime_enabled;
                if old_runtime_enabled {
                    let (finished, completion) = mpsc::sync_channel(1);
                    self.retiring_runtime = Some(completion);
                    let retired = RetiredEngine {
                        engine: old_engine,
                        finished,
                        waker: self.waker.clone(),
                    };
                    if let Err(error) = self.retired_engine_sender.send(retired) {
                        // The reaper only exits when ReloadableConfig is
                        // dropped. A disconnected channel therefore means a
                        // broken reaper; keep teardown correct and surface
                        // the exceptional synchronous fallback.
                        warn!("engine reaper stopped unexpectedly; disposing old engine inline");
                        drop(error.0.engine);
                        let _ = error.0.finished.send(());
                    }
                } else {
                    drop(old_engine);
                }
                if !self.ensure_async_workers() {
                    warn!(
                        "command workers could not restart after configuration reload; command-backed actions are disabled until old workers exit or a retry succeeds"
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

    /// Operational state for status and user-facing diagnostics. A config
    /// path switch is reported as loading until its current target finishes;
    /// an invalid completed load remains distinctly rejected.
    pub fn status_state(&self) -> &'static str {
        if self.healthy {
            "ok"
        } else if self.rebuild_after_in_flight
            || self
                .in_flight
                .as_ref()
                .is_some_and(|load| load.path == self.path)
        {
            "loading"
        } else {
            "reload-rejected"
        }
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

fn load_consistent(path: &Path) -> Result<(Config, Option<FileStamp>, String)> {
    for _attempt in 0..MAX_CONSISTENCY_ATTEMPTS {
        let before = file_stamp(path);
        let content_before = file_content_fingerprint(path);
        let loaded = Config::load_versioned(path)?;
        let source = loaded.source().to_owned();
        let config = loaded.config;
        let after = file_stamp(path);
        let content_after = file_content_fingerprint(path);
        if before == after && content_before == content_after {
            return Ok((config, after, source));
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
        let (mut base, stamp, base_source) = load_consistent(path)?;
        if fleet {
            let before = standard_fleet_content_signature();
            // Kept typed so `safe_reload_error` can report what went wrong.
            let merged =
                FleetConfig::load_standard_with_base_source_and_policy(base, &base_source, policy)
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
mod tests;
