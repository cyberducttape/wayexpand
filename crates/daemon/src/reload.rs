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
        Arc,
    },
    time::{Duration, Instant, SystemTime},
};
use tracing::{error, info, warn};
use wayexpand_core::{
    Config, ConfigError, ExpansionEngine, FleetConfig, FleetError, OrganizationPolicy,
};

const MAX_CONSISTENCY_ATTEMPTS: usize = 3;
const INTEGRITY_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

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
    let fingerprint = stable_content_fingerprint(&contents);
    Some(FileStamp {
        modified,
        length,
        inode,
        change_time,
        change_time_nsec,
        fingerprint,
    })
}

fn stable_content_fingerprint(contents: &[u8]) -> u64 {
    // FNV-1a is deterministic across compiler versions and sensitive to byte
    // order, unlike the previous commutative chunk sum.
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
            fleet_signature: standard_fleet_signature(),
            policy,
            engine,
            healthy: true,
        })
    }

    /// Parse first, then replace the live engine. Invalid edits leave the old
    /// configuration running and are reported to the operator.
    pub fn reload_if_changed(&mut self) {
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
        let current = file_stamp(&self.path);
        self.last_integrity_check = Instant::now();
        self.watch_check_after = None;
        self.watch_dirty.store(false, Ordering::Release);
        self.observed = current;
        self.reload_current(current);
    }

    fn reload_current(&mut self, current: Option<FileStamp>) {
        match load_for_mode(&self.path, self.fleet, &self.policy) {
            Ok((config, stable_stamp)) => {
                let count = config.expansion.len();
                match ExpansionEngine::new(config) {
                    Ok(mut engine) => {
                        let workers_restart_failed =
                            self.engine.async_commands_enabled() && !engine.enable_async_commands();
                        if workers_restart_failed {
                            warn!(
                                "asynchronous workers could not restart after configuration reload; command-backed actions are disabled"
                            );
                        }
                        engine.set_commands_disabled(
                            self.engine.commands_disabled() || workers_restart_failed,
                        );
                        engine.set_title_matching_disabled(self.engine.title_matching_disabled());
                        engine.set_reinsert_terminators(self.engine.reinserts_terminators());
                        // Keep waking the reactor when commands finish.
                        engine.set_completion_notifier(self.engine.completion_notifier());
                        engine.set_clipboard_reader(self.engine.clipboard_reader());
                        engine.set_clipboard_prefetch(self.engine.clipboard_prefetch());
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
                        engine.set_composition_active(self.engine.is_composition_active());
                        self.engine = engine;
                        self.stamp = stable_stamp;
                        self.observed = stable_stamp;
                        self.last_integrity_check = Instant::now();
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

fn load_for_mode(
    path: &Path,
    fleet: bool,
    policy: &OrganizationPolicy,
) -> Result<(Config, Option<FileStamp>)> {
    let (mut base, stamp) = load_consistent(path)?;
    if fleet {
        // Kept typed so `safe_reload_error` can report what went wrong.
        let merged = FleetConfig::load_standard_with_base_and_policy(base, policy)
            .map_err(anyhow::Error::new)?;
        for violation in &merged.policy_violations {
            super::policy::log_violation(policy, violation);
        }
        base = merged.config;
    }

    base.apply_administrator_policy(policy)
        .map_err(anyhow::Error::new)?;

    Ok((base, stamp))
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
        if let Ok(contents) = fs::read(&path) {
            contents.hash(&mut hasher);
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
    fn fleet_signature_tracks_nested_pack_file_content() {
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

        let before = fleet_signature_for([nested.clone()]);
        fs::write(&nested, "after!").unwrap();
        let after = fleet_signature_for([nested.clone()]);

        assert_ne!(before, after);
        fs::remove_dir_all(root).unwrap();
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
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
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
        let mut config =
            ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
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
