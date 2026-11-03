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

fn wait_for_command_workers(config: &mut ReloadableConfig) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !config.engine.async_commands_enabled() {
        assert!(Instant::now() < deadline, "command workers did not restart");
        config.retry_async_workers();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!config.engine.commands_disabled());
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
fn profile_path_switch_discards_an_obsolete_in_flight_library() {
    let (directory, base_path) = isolated_temporary_config();
    let profiles = directory.join("profiles");
    fs::create_dir(&profiles).unwrap();
    fs::set_permissions(&profiles, fs::Permissions::from_mode(0o700)).unwrap();
    let obsolete = profiles.join("work.toml");
    let selected = profiles.join("support.toml");
    write_config(&base_path, &config_text("base"));
    write_config(&obsolete, &config_text("work"));
    write_config(&selected, &config_text("support"));
    let mut config =
        ReloadableConfig::load_with_policy(&base_path, OrganizationPolicy::default()).unwrap();

    config.switch_path(obsolete);
    config.switch_path(selected.clone());
    assert_eq!(config.status_state(), "loading");
    config.finish_pending_reload();

    assert_eq!(config.path(), selected);
    assert!(config.healthy());
    assert_eq!(config.status_state(), "ok");
    let result = config
        .engine
        .process(InputEvent::Text(":x".into()))
        .pop()
        .unwrap();
    assert_eq!(result.insert, "support");

    let invalid = profiles.join("invalid.toml");
    write_config(&invalid, "not valid = [toml");
    config.switch_path(invalid);
    assert_eq!(config.status_state(), "loading");
    config.finish_pending_reload();
    assert_eq!(config.status_state(), "reload-rejected");
    fs::remove_dir_all(directory).unwrap();
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
        wait_for_command_workers(&mut config);
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
    wait_for_command_workers(&mut config);
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
    let deadline = Instant::now() + Duration::from_secs(2);
    while !config.engine.async_commands_enabled() {
        assert!(
            Instant::now() < deadline,
            "restricted workers did not restart"
        );
        config.retry_async_workers();
        thread::sleep(Duration::from_millis(2));
    }
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

    // Hand the reactor a completed first-generation build that has not
    // been collected yet. This makes the reload overlap deterministic:
    // the next request must queue a rebuild instead of racing a worker
    // thread that might finish before the second edit.
    let stale_build = build_config(&path, false, &config.policy)
        .unwrap_or_else(|_| panic!("fixture configuration must build"));
    let requested_stamp = file_stamp(&path);
    let (sender, result) = mpsc::sync_channel(1);
    assert!(sender.send(Ok(stale_build)).is_ok());
    config.in_flight = Some(InFlightReload {
        result,
        path: path.clone(),
        current: requested_stamp,
        fleet_signature: 0,
    });

    write_config(&path, &config_text("second"));
    config.reload_now();
    assert!(config.rebuild_after_in_flight);
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
fn a_completed_stale_snapshot_is_not_promoted() {
    let path = temporary_config();
    write_config(&path, &config_text("old"));
    let mut config =
        ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();

    write_config(&path, &config_text("first"));
    let stale_build = build_config(&path, false, &config.policy)
        .unwrap_or_else(|_| panic!("fixture configuration must build"));
    let requested_stamp = file_stamp(&path);
    let (sender, result) = mpsc::sync_channel(1);
    assert!(sender.send(Ok(stale_build)).is_ok());
    config.in_flight = Some(InFlightReload {
        result,
        path: path.clone(),
        current: requested_stamp,
        fleet_signature: 0,
    });

    // The worker has completed a valid build, but the source changes
    // before the reactor gets a chance to collect and promote it.
    write_config(&path, &config_text("second"));
    config.collect_reload();
    assert!(config.in_flight.is_some());
    let still_active = config
        .engine
        .process(InputEvent::Text(":x".into()))
        .pop()
        .unwrap();
    assert_eq!(still_active.insert, "old");

    config.finish_pending_reload();
    let current = config
        .engine
        .process(InputEvent::Text(":x".into()))
        .pop()
        .unwrap();
    assert_eq!(current.insert, "second");
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
fn only_changes_to_configuration_paths_request_a_rescan() {
    use notify::event::{AccessKind, AccessMode, CreateKind, EventKind, ModifyKind};
    let filter = WatchFilter {
        files: vec![PathBuf::from("/c/config.toml")],
        trees: vec![PathBuf::from("/c/snippets.d")],
    };
    let event = |kind, path: &str| Ok(notify::Event::new(kind).add_path(path.into()));
    let config = "/c/config.toml";
    let rescan = |event| event_requests_rescan(&event, &filter);
    assert!(!rescan(event(
        EventKind::Access(AccessKind::Open(AccessMode::Any)),
        config
    )));
    assert!(!rescan(event(
        EventKind::Access(AccessKind::Close(AccessMode::Read)),
        config
    )));
    assert!(rescan(event(
        EventKind::Access(AccessKind::Close(AccessMode::Write)),
        config
    )));
    assert!(rescan(event(EventKind::Modify(ModifyKind::Any), config)));
    assert!(rescan(event(
        EventKind::Create(CreateKind::File),
        "/c/snippets.d/work.toml"
    )));
    // Other files in the watched directory, such as usage statistics.
    assert!(!rescan(event(
        EventKind::Modify(ModifyKind::Any),
        "/c/usage-stats.json"
    )));
    assert!(!rescan(event(
        EventKind::Create(CreateKind::File),
        "/c/.config.toml.tmp.1"
    )));
    // Unattributable events and lost events still rescan.
    assert!(rescan(Ok(notify::Event::new(EventKind::Any))));
    assert!(rescan(Err(notify::Error::generic("lost"))));
}

/// A private watched directory: other tests writing their configs into the
/// shared temporary directory must not be visible to this one. It is 0700
/// because the loader refuses a group-writable parent.
fn watched_test_config() -> (PathBuf, PathBuf, ReloadableConfig) {
    let directory = temporary_config().with_extension("d");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("config.toml");
    write_config(&path, &config_text("old"));
    let config = ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
    // Let events from the initial write and load drain.
    thread::sleep(Duration::from_millis(200));
    config.watch_dirty.store(false, Ordering::Release);
    (directory, path, config)
}

#[test]
fn reading_the_watched_config_does_not_mark_it_dirty() {
    let (directory, path, config) = watched_test_config();
    if config._watcher.is_some() {
        let _ = fs::read(&path).unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(!config.watch_dirty.load(Ordering::Acquire));
    }
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn writing_a_sibling_file_does_not_mark_the_config_dirty() {
    let (directory, path, config) = watched_test_config();
    if config._watcher.is_some() {
        fs::write(directory.join("usage-stats.json"), "{}").unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(!config.watch_dirty.load(Ordering::Acquire));

        write_config(&path, &config_text("new"));
        thread::sleep(Duration::from_millis(200));
        assert!(config.watch_dirty.load(Ordering::Acquire));
    }
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn watcher_event_reloads_when_metadata_stamp_is_unchanged() {
    let path = temporary_config();
    write_config(&path, &config_text("old"));
    let mut config =
        ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();

    // Model a filesystem whose timestamp granularity leaves the metadata
    // stamp unchanged after an in-place write. The watcher notification
    // must still cause a background read and validated engine swap.
    write_config(&path, &config_text("new"));
    config.observed = file_stamp(&path);
    config.watch_check_after = Some(Instant::now() - WATCH_DEBOUNCE);
    config.watch_dirty.store(true, Ordering::Release);
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
        config.retry_async_workers();
        assert!(config.engine.drain_completed_commands().is_empty());
        std::thread::sleep(Duration::from_millis(20));
    }

    // The reloaded engine still runs commands.
    wait_for_command_workers(&mut config);
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
fn reload_does_not_wait_for_a_blocked_command_worker_on_the_reactor() {
    let slow = "[[expansion]]\ntrigger = \":slow\"\nreplacement = \"\"\n\
                    [expansion.command]\nprogram = \"/bin/sh\"\n\
                    args = [\"-c\", \"sleep 10; printf stale-output\"]\ntimeout_ms = 5000\n";
    let path = temporary_config();
    write_config(&path, &format!("{slow}{}", config_text("before")));
    let mut config =
        ReloadableConfig::load_with_policy(&path, OrganizationPolicy::default()).unwrap();
    assert!(config.start_async_workers(false));
    assert!(config
        .engine
        .process(InputEvent::Text(":slow".into()))
        .is_empty());

    let deadline = Instant::now() + Duration::from_secs(2);
    while config.engine.command_metrics().command_in_flight == 0 {
        assert!(Instant::now() < deadline, "blocked command did not start");
        thread::sleep(Duration::from_millis(2));
    }

    write_config(&path, &config_text("reloaded"));
    config.reload_now();
    let mut slowest_reactor_turn = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_secs(2);
    while config.in_flight.is_some() {
        let turn_started = Instant::now();
        config.collect_reload();
        slowest_reactor_turn = slowest_reactor_turn.max(turn_started.elapsed());
        assert!(Instant::now() < deadline, "reload did not finish");
        if config.in_flight.is_some() {
            thread::sleep(Duration::from_millis(1));
        }
    }
    assert!(
        slowest_reactor_turn < Duration::from_millis(150),
        "reload application blocked the input reactor for {slowest_reactor_turn:?}"
    );

    let input_started = Instant::now();
    let typed = config.engine.process(InputEvent::Text(":x".into()));
    assert!(
        input_started.elapsed() < Duration::from_millis(150),
        "input processing waited for old worker teardown"
    );
    assert_eq!(typed[0].insert, "reloaded");
    wait_for_command_workers(&mut config);
    assert!(config.engine.drain_completed_commands().is_empty());
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
