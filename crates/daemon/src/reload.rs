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
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use tracing::{error, info, warn};
use wayexpand_core::{
    limits::MAX_CONFIG_BYTES, Config, ConfigError, ExpansionEngine, FleetConfig, FleetError,
    InjectorCapabilities, InputSourceCapabilities, OrganizationPolicy,
};

const MAX_CONSISTENCY_ATTEMPTS: usize = 3;
const INTEGRITY_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);
/// How often command workers that could not start are tried again.
const WORKER_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub(crate) struct ConfigDiagnostics {
    pub active_path: String,
    pub generation: u64,
    pub loaded_at_unix: u64,
    pub last_successful_reload_unix: u64,
    pub enabled_expansions: usize,
    pub disabled_expansions: usize,
    pub generate_only_expansions: usize,
    pub last_reload_failure: String,
    pub validation_errors: String,
}

fn diagnostics_store() -> &'static Mutex<ConfigDiagnostics> {
    static STORE: OnceLock<Mutex<ConfigDiagnostics>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(ConfigDiagnostics::default()))
}

pub(crate) fn diagnostics_snapshot() -> ConfigDiagnostics {
    diagnostics_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn record_initial_config(path: &Path, enabled: usize, disabled: usize, generate_only: usize) {
    let now = unix_now();
    let mut state = diagnostics_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.generation = state.generation.saturating_add(1).max(1);
    state.active_path = path.display().to_string();
    state.loaded_at_unix = now;
    state.last_successful_reload_unix = now;
    state.enabled_expansions = enabled;
    state.disabled_expansions = disabled;
    state.generate_only_expansions = generate_only;
    state.validation_errors.clear();
}

fn record_reload_success(path: &Path, enabled: usize, disabled: usize, generate_only: usize) {
    let now = unix_now();
    let mut state = diagnostics_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.generation = state.generation.saturating_add(1).max(1);
    state.active_path = path.display().to_string();
    state.loaded_at_unix = now;
    state.last_successful_reload_unix = now;
    state.enabled_expansions = enabled;
    state.disabled_expansions = disabled;
    state.generate_only_expansions = generate_only;
    state.validation_errors.clear();
}

fn record_reload_failure(error: &anyhow::Error) {
    let message = safe_reload_error(error);
    let mut state = diagnostics_store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.last_reload_failure = message.clone();
    state.validation_errors = message;
}

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
    route_injector: InjectorCapabilities,
    route_source: InputSourceCapabilities,
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
    enabled_expansions: usize,
    disabled_expansions: usize,
    generate_only_expansions: usize,
    stamp: Option<FileStamp>,
    /// Fleet metadata as it existed before this load began. If a layer is
    /// edited while the build is running or before the reactor applies it,
    /// the next integrity check must still observe the change.
    fleet_signature: u64,
}

/// Whether a completed reload result still describes the active request and
/// source state at the moment the reactor is about to publish it. Successful
/// builds are checked against the snapshot they actually read; failed builds
/// are relevant only if the original request's snapshot is still current.
/// Keeping this decision in one predicate makes stale-result admission easy
/// to exercise independently from filesystem timing.
fn reload_snapshot_is_current<S: PartialEq>(
    request_targets_active_path: bool,
    requested_stamp: &S,
    current_stamp: &S,
    requested_fleet_signature: u64,
    current_fleet_signature: u64,
    built_snapshot: Option<(&S, u64)>,
) -> bool {
    if !request_targets_active_path {
        return false;
    }

    match built_snapshot {
        Some((built_stamp, built_fleet_signature)) => {
            built_stamp == current_stamp && built_fleet_signature == current_fleet_signature
        }
        None => {
            requested_stamp == current_stamp && requested_fleet_signature == current_fleet_signature
        }
    }
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
        let enabled_expansions = config
            .expansion
            .iter()
            .filter(|entry| entry.is_keyboard_matchable())
            .count();
        let disabled_expansions = config
            .expansion
            .iter()
            .filter(|entry| !entry.enabled)
            .count();
        let generate_only_expansions = config
            .expansion
            .iter()
            .filter(|entry| {
                entry.enabled && entry.delivery == wayexpand_core::ExpansionDelivery::GenerateOnly
            })
            .count();
        let snapshot = config
            .validate_snapshot()
            .map_err(|error| BuildFailure::Engine(anyhow::Error::new(error)))?;
        warn_if_undo_unavailable(&config);
        let engine = ExpansionEngine::from_validated(Arc::new(snapshot));
        return Ok(BuiltConfig {
            engine,
            expansions,
            enabled_expansions,
            disabled_expansions,
            generate_only_expansions,
            stamp,
            fleet_signature: fleet_after,
        });
    }
    unreachable!("fleet consistency loop returns or fails on every attempt")
}

/// Disable automatic expansion on `engine` while the active route cannot
/// satisfy its effective organization policy.
fn apply_backend_policy_gate(
    engine: &mut ExpansionEngine,
    route_injector: InjectorCapabilities,
    route_source: InputSourceCapabilities,
) {
    let violation = engine.capability_violation_for_source(route_injector, route_source);
    engine.set_backend_policy_blocked(violation.is_some());
    if let Some(violation) = violation {
        warn!(reason = %violation, "active backend cannot satisfy effective policy; automatic expansion is disabled");
    }
}

/// No current daemon output route can verify that the application applied
/// an expansion, so the daemon never records undo state. Say so rather than
/// leaving a configured `undo_chord` silently inert.
fn warn_if_undo_unavailable(config: &wayexpand_core::Config) {
    if config.settings.undo_chord.is_some() {
        warn!(
            "undo_chord is set, but expansion undo is unavailable: no current output route can verify the application text it would undo"
        );
    }
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
        let enabled_expansions = config
            .expansion
            .iter()
            .filter(|entry| entry.is_keyboard_matchable())
            .count();
        let disabled_expansions = config
            .expansion
            .iter()
            .filter(|entry| !entry.enabled)
            .count();
        let generate_only_expansions = config
            .expansion
            .iter()
            .filter(|entry| {
                entry.enabled && entry.delivery == wayexpand_core::ExpansionDelivery::GenerateOnly
            })
            .count();
        let snapshot = config
            .validate_snapshot()
            .map_err(|error| anyhow::anyhow!("invalid configuration: {error}"))?;
        warn_if_undo_unavailable(&config);
        let engine = ExpansionEngine::from_validated(Arc::new(snapshot));
        record_initial_config(
            &path,
            enabled_expansions,
            disabled_expansions,
            generate_only_expansions,
        );
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
            route_injector: InjectorCapabilities::default(),
            route_source: InputSourceCapabilities::default(),
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

    /// Store the negotiated route contract and apply it to the active policy.
    /// Every future reload is checked against this same live route before its
    /// engine is published.
    pub fn set_route_capabilities(
        &mut self,
        injector: InjectorCapabilities,
        source: InputSourceCapabilities,
    ) {
        self.route_injector = injector;
        self.route_source = source;
        self.refresh_backend_policy_gate();
    }

    fn refresh_backend_policy_gate(&mut self) {
        apply_backend_policy_gate(&mut self.engine, self.route_injector, self.route_source);
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
                    if !self.engine.requires_async_workers() {
                        self.worker_retry_at = None;
                        return true;
                    }
                    self.engine
                        .set_commands_disabled(self.commands_restricted || self.async_workers);
                    self.worker_retry_at = None;
                    return false;
                }
            }
        }
        if !self.engine.requires_async_workers() {
            self.worker_retry_at = None;
            return true;
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
            let built_snapshot = built
                .as_ref()
                .ok()
                .map(|built| (&built.stamp, built.fleet_signature));
            if reload_snapshot_is_current(
                true,
                &current,
                &current_stamp,
                requested_fleet_signature,
                current_fleet_signature,
                built_snapshot,
            ) {
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
                enabled_expansions,
                disabled_expansions,
                generate_only_expansions,
                stamp: stable_stamp,
                fleet_signature,
            }) => {
                // Pause, password-field, composition and window state,
                // command restrictions and host hooks all carry over, as of
                // now rather than when the build started; losing any of them
                // would, for example, resume matching inside a password field.
                engine.inherit_runtime_state(&self.engine);
                // Gate the new engine before it replaces the old one, so no
                // turn ever runs it ungated.
                apply_backend_policy_gate(&mut engine, self.route_injector, self.route_source);
                // A host that enabled workers on the engine directly wants
                // them after the reload too.
                self.async_workers |= self.engine.async_commands_enabled();
                // The old runtime's Drop joins its command and hotkey workers.
                // Hand it to a reaper so a blocked cancellation cannot stall
                // this input-reactor turn. New workers remain disabled until
                // the reaper confirms the old set has exited.
                let old_engine = std::mem::replace(&mut self.engine, engine);
                crate::status::set_raw_capture_fallback(
                    self.engine.raw_capture_fallback_enabled(),
                    self.engine.raw_capture_app_count(),
                );
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
                record_reload_success(
                    &self.path,
                    enabled_expansions,
                    disabled_expansions,
                    generate_only_expansions,
                );
                info!(expansions, "configuration reloaded");
            }
            Err(BuildFailure::Engine(error)) => {
                self.healthy = false;
                record_reload_failure(&error);
                error!(
                    reason = %safe_reload_error(&error),
                    "configuration reload rejected; keeping previous configuration"
                )
            }
            Err(BuildFailure::Load(error)) => {
                self.healthy = false;
                record_reload_failure(&error);
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

/// Scheduler model for the reload/async-completion handoff. ReloadableConfig
/// promotes engines and drains completions on the same reactor; each engine
/// owns its completion queue, so a retired engine's result cannot be consumed
/// by the replacement engine. The model uses separate per-epoch mailboxes to
/// exercise that ownership boundary while invoking the production reload
/// snapshot admission predicate.
#[cfg(all(test, feature = "loom-tests"))]
mod reload_async_loom {
    use loom::sync::{Arc, Mutex};
    use loom::thread;

    use super::reload_snapshot_is_current;

    #[derive(Default)]
    struct ReactorState {
        active_epoch: u8,
        promoted: bool,
        applied: Option<(u8, u8, bool)>,
    }

    #[test]
    fn reload_promotion_never_applies_a_retired_engine_completion() {
        loom::model(|| {
            // The build and current file both describe snapshot 2. Stale file
            // and fleet snapshots are exhaustively tested against this same
            // production predicate by reload_admission_exhaustively... below.
            let old_engine_completion = Arc::new(loom::sync::atomic::AtomicBool::new(false));
            let replacement_engine_completion =
                Arc::new(loom::sync::atomic::AtomicBool::new(false));
            let reactor = Arc::new(Mutex::new(ReactorState {
                active_epoch: 1,
                ..ReactorState::default()
            }));

            let completion_mailbox = Arc::clone(&old_engine_completion);
            let command_worker = thread::spawn(move || {
                thread::yield_now();
                // Completion remains in the mailbox owned by engine epoch 1.
                completion_mailbox.store(true, loom::sync::atomic::Ordering::Release);
            });

            let promotion_state = Arc::clone(&reactor);
            let promoter = thread::spawn(move || {
                thread::yield_now();
                // Reload promotion and completion draining are serialized by
                // the production input reactor. File metadata can still
                // change independently while the predicate is evaluated.
                let mut state = promotion_state.lock().unwrap();
                let still_current =
                    reload_snapshot_is_current(true, &2_u8, &2_u8, 4, 4, Some((&2_u8, 4)));
                if still_current {
                    state.active_epoch = 2;
                    state.promoted = true;
                }
            });

            let drain_state = Arc::clone(&reactor);
            let old_mailbox = Arc::clone(&old_engine_completion);
            let new_mailbox = Arc::clone(&replacement_engine_completion);
            let drainer = thread::spawn(move || {
                thread::yield_now();
                let mut state = drain_state.lock().unwrap();
                let active_epoch = state.active_epoch;
                let pending = match active_epoch {
                    1 => old_mailbox.swap(false, loom::sync::atomic::Ordering::AcqRel),
                    2 => new_mailbox.swap(false, loom::sync::atomic::Ordering::AcqRel),
                    _ => unreachable!("model has exactly two runtime epochs"),
                };
                if pending {
                    state.applied = Some((1, active_epoch, state.promoted));
                }
            });

            command_worker.join().unwrap();
            promoter.join().unwrap();
            drainer.join().unwrap();

            let state = reactor.lock().unwrap();
            if let Some((completion_epoch, applied_epoch, promoted_before_apply)) = state.applied {
                assert_eq!(completion_epoch, applied_epoch);
                assert!(
                    !promoted_before_apply,
                    "a retired engine completion was applied after reload promotion"
                );
            }
            if state.promoted {
                assert_eq!(state.active_epoch, 2);
                assert!(
                    !replacement_engine_completion.load(loom::sync::atomic::Ordering::Acquire),
                    "the replacement runtime must not inherit the retired completion"
                );
            }
        });
    }

    /// Compose the boundaries exercised by the individual models: an old
    /// output is in flight while the reactor changes target/consent, publishes
    /// a new engine epoch, and starts shutdown. A late worker completion must
    /// neither mutate the new engine nor make the uncertain output replayable.
    #[test]
    fn output_reload_focus_and_shutdown_interleaving_quarantines_stale_work() {
        use loom::sync::Mutex;
        use wayexpand_core::FocusCheck;

        use crate::{
            assurance::tracked_output_baseline_is_valid,
            pending_output::{outcome_invalidates_context, OutputIntegrityGate},
        };

        const AUTHORIZED_MODE: u64 = (1 << 2) | 1;
        const REVOKED_AND_REGRANTED_MODE: u64 = (3 << 2) | 1;

        #[derive(Default)]
        struct State {
            active_epoch: u8,
            focus_generation: u8,
            mode_state: u64,
            shutdown: bool,
            output_events: usize,
            output_events_after_invalidation: usize,
            worker_completion_epoch: Option<u8>,
            completion_applied: bool,
            command_completion_epoch: Option<u8>,
            command_completion_applied: bool,
            partial_injector_failure_seen: bool,
            partial_failure_invalidates_context: bool,
            quarantined: bool,
            replay_count: usize,
            integrity: OutputIntegrityGate,
        }

        // The additional command worker adds an independent completion
        // actor. Bound scheduler preemptions while retaining exhaustive
        // coverage of every transition point in this compact model.
        let mut builder = loom::model::Builder::new();
        builder.preemption_bound = Some(3);
        builder.max_branches = 10_000;
        builder.check(|| {
            let state = Arc::new(Mutex::new(State {
                active_epoch: 1,
                focus_generation: 1,
                mode_state: AUTHORIZED_MODE,
                ..State::default()
            }));

            // This worker models a paced two-unit stream. There is a gap
            // between each checkpoint and backend call. Invalidation may
            // race with one admitted call but must stop the next checkpoint;
            // already-emitted text cannot be retracted.
            let worker_state = Arc::clone(&state);
            let worker = thread::spawn(move || {
                for _unit in 0..2 {
                    let admitted = {
                        let state = worker_state.lock().unwrap();
                        let focus = if state.focus_generation == 1 {
                            FocusCheck::Unchanged
                        } else {
                            FocusCheck::Changed
                        };
                        !state.shutdown && tracked_output_baseline_is_valid(state.mode_state, focus)
                    };
                    if !admitted {
                        break;
                    }
                    thread::yield_now();

                    let mut state = worker_state.lock().unwrap();
                    state.output_events += 1;
                    if state.shutdown
                        || state.focus_generation != 1
                        || state.mode_state != AUTHORIZED_MODE
                    {
                        state.output_events_after_invalidation += 1;
                        state.integrity.interrupt_for_physical_input();
                    }
                }

                let mut state = worker_state.lock().unwrap();
                if state.output_events > 0 {
                    // Model the injector failing after a previously submitted
                    // output unit. The same production classifier used by the
                    // reactor must make this result non-replayable.
                    let outcome = wayexpand_core::TransactionOutcome::UnknownPartialFailure {
                        source: wayexpand_core::InjectorError::input_interrupted("loom-model"),
                    };
                    state.partial_injector_failure_seen = true;
                    state.partial_failure_invalidates_context =
                        outcome_invalidates_context(&outcome);
                }
                // Completion belongs to runtime epoch 1 even if epoch 2 has
                // already been promoted by the reactor.
                state.worker_completion_epoch = Some(1);
            });

            // A command was admitted by the old engine before reload. Its
            // completion remains tagged with that engine epoch even if it
            // races the output worker and the reactor's reload transition.
            let command_state = Arc::clone(&state);
            let command_worker = thread::spawn(move || {
                thread::yield_now();
                command_state.lock().unwrap().command_completion_epoch = Some(1);
            });

            let reactor_state = Arc::clone(&state);
            let reactor = thread::spawn(move || {
                thread::yield_now();
                let mut state = reactor_state.lock().unwrap();

                // A→B→A still changes the identity generation; revoking then
                // restoring consent also has a distinct generation.
                state.focus_generation += 1;
                state.mode_state = REVOKED_AND_REGRANTED_MODE;
                state.integrity.interrupt_for_physical_input();

                // Promote a validated replacement configuration and request
                // shutdown while the previous worker may still be finishing.
                let reload_is_current =
                    reload_snapshot_is_current(true, &2_u8, &2_u8, 9, 9, Some((&2_u8, 9)));
                if reload_is_current {
                    state.active_epoch = 2;
                }
                state.shutdown = true;

                if state.worker_completion_epoch == Some(state.active_epoch) {
                    state.completion_applied = true;
                }
                if state.command_completion_epoch == Some(state.active_epoch) {
                    state.command_completion_applied = true;
                }
            });

            worker.join().unwrap();
            command_worker.join().unwrap();
            reactor.join().unwrap();

            let mut state = state.lock().unwrap();
            assert_eq!(state.active_epoch, 2);
            assert_eq!(state.worker_completion_epoch, Some(1));
            assert!(
                !state.completion_applied,
                "retired completion crossed the reload epoch"
            );
            assert_eq!(state.command_completion_epoch, Some(1));
            assert!(
                !state.command_completion_applied,
                "a pending command completion crossed the reload epoch"
            );
            if state.partial_injector_failure_seen {
                assert!(
                    state.partial_failure_invalidates_context,
                    "partial injector failure must invalidate output context"
                );
            }
            // Resolve only after the output worker's terminal result is
            // published. A partial injector failure uses the production
            // classifier, while focus/consent/shutdown invalidation remains
            // independently terminal.
            let invalidate_context = state.partial_failure_invalidates_context
                || state.focus_generation != 1
                || state.mode_state != AUTHORIZED_MODE
                || state.shutdown;
            state.quarantined = state.integrity.resolve(invalidate_context);
            let continuation = crate::pending_output::output_continuation(
                crate::pending_output::OutputOrigin::Command,
                state.quarantined,
                false,
            );
            if continuation == crate::pending_output::OutputContinuation::ReplayFollowUp {
                state.replay_count += 1;
            }
            if state.partial_injector_failure_seen {
                assert_eq!(
                    continuation,
                    crate::pending_output::OutputContinuation::Quarantine,
                    "uncertain command output must never be replayed"
                );
            }
            assert!(
                state.quarantined,
                "invalidated output must quarantine the target context"
            );
            assert_eq!(
                state.replay_count, 0,
                "uncertain output must never be replayed"
            );
            assert!(
                state.output_events_after_invalidation <= 1,
                "only one backend call admitted before invalidation may still be in flight"
            );
        });
    }
}
