use super::{
    telemetry::{self, IbusTelemetry},
    IbusAction, IbusEngineAdapter,
};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
use tracing::{info, warn};
use wayexpand_backend_gnome_window::GnomeWindowTracker;
use wayexpand_core::{
    default_config_path, Config, ConfigStore, OrganizationPolicy, ValidatedConfig, WindowContext,
    WindowTracker,
};
use zbus::{
    blocking::{connection::Builder, Connection},
    interface,
    zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value},
};

/// Must equal the component `<name>` in `desktop/wayexpand-ibus.xml`:
/// ibus-daemon launches the component's `exec`, then waits for this exact
/// name to gain an owner before it sends `CreateEngine` to the factory. Any
/// other name leaves engine selection hanging until IBus times out.
pub(crate) const BUS_NAME: &str = "org.freedesktop.IBus.WayExpand";
const FACTORY_PATH: &str = "/org/freedesktop/IBus/Factory";
const ENGINE_PATH: &str = "/org/freedesktop/IBus/Engine/WayExpand";

#[derive(Debug, thiserror::Error)]
pub enum IbusServiceError {
    #[error("could not load WayExpand configuration: {0}")]
    Config(#[from] wayexpand_core::ConfigError),
    #[error("D-Bus service failed: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("could not start configuration watcher: {0}")]
    Thread(String),
    #[error("organization policy is invalid: {0}")]
    Policy(String),
}

/// Feed only authenticated GNOME Shell focus updates to IBus engines. A
/// tracker error clears the snapshot before reconnecting, so consumers
/// immediately lose app identity instead of retaining stale focus metadata.
fn spawn_gnome_focus_tracker(
    focus: Arc<Mutex<Option<WindowContext>>>,
) -> Result<(), IbusServiceError> {
    std::thread::Builder::new()
        .name("wayexpand-ibus-gnome-focus".into())
        .spawn(move || loop {
            match GnomeWindowTracker::new() {
                Ok(mut tracker) => loop {
                    match tracker.next_window_timeout(Duration::from_millis(250)) {
                        Ok(Some(window)) => {
                            if let Ok(mut current) = focus.lock() {
                                *current = window;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            warn!(%error, "GNOME focus tracking unavailable to IBus; app-filtered expansions fail closed");
                            if let Ok(mut current) = focus.lock() {
                                *current = None;
                            }
                            break;
                        }
                    }
                },
                Err(error) => {
                    warn!(%error, "GNOME focus tracking unavailable to IBus; app-filtered expansions fail closed");
                    if let Ok(mut current) = focus.lock() {
                        *current = None;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        })
        .map_err(|error| IbusServiceError::Thread(error.to_string()))?;
    Ok(())
}

/// IBus factory state. Each CreateEngine call gets its own adapter and object
/// path; IBus may create multiple engines for separate input contexts. Engine
/// objects are removed from this map by their Destroy method.
struct Factory {
    connection: Arc<Mutex<Option<Connection>>>,
    validated_config: Arc<Mutex<Arc<ValidatedConfig>>>,
    config_generation: Arc<AtomicU64>,
    policy: Arc<OrganizationPolicy>,
    instances: Arc<Mutex<HashMap<OwnedObjectPath, EngineInstance>>>,
    next_id: AtomicU64,
    /// Wakes the completion pump with the path of an engine whose
    /// asynchronous command finished.
    completions: mpsc::Sender<OwnedObjectPath>,
    /// Content-free activity shared by every engine of this process.
    telemetry: Option<Arc<IbusTelemetry>>,
    gnome_focus: Arc<Mutex<Option<WindowContext>>>,
}

type Instances = Arc<Mutex<HashMap<OwnedObjectPath, EngineInstance>>>;

struct EngineInstance {
    adapter: Arc<Mutex<IbusEngineAdapter>>,
    config_generation: AtomicU64,
}

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    /// Async so the object is registered through the async object server.
    /// Packaged builds share zbus with crates that enable its tokio backend,
    /// where a blocking zbus call from inside a handler panics ("cannot start
    /// a runtime from within a runtime") and the engine is never created.
    async fn create_engine(&self, name: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if name != "wayexpand" && name != "WayExpand" {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "unsupported WayExpand engine name: {name}"
            )));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let path = engine_path(id);
        let (validated, generation) = {
            let current = self
                .validated_config
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("IBus config lock poisoned".into()))?;
            (
                Arc::clone(&current),
                self.config_generation.load(Ordering::Acquire),
            )
        };
        let mut adapter =
            IbusEngineAdapter::with_validated_config(validated, (*self.policy).clone());
        let completions = self.completions.clone();
        let completed_path = path.clone();
        adapter.set_completion_notifier(Some(Arc::new(move || {
            let _ = completions.send(completed_path.clone());
        })));
        if let Some(telemetry) = &self.telemetry {
            adapter.set_telemetry(Arc::clone(telemetry));
        }
        let adapter = Arc::new(Mutex::new(adapter));
        let engine = EngineObject {
            adapter: Arc::clone(&adapter),
            connection: Arc::clone(&self.connection),
            path: path.clone(),
            instances: Arc::clone(&self.instances),
            telemetry: self.telemetry.clone(),
            gnome_focus: Arc::clone(&self.gnome_focus),
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus connection lock poisoned".into()))?
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("IBus connection unavailable".into()))?;
        {
            // Lock order is config snapshot, then instances, then adapter.
            // Reload uses the same order. Re-read after adapter construction
            // so a context racing a reload is brought to the current
            // generation before it becomes visible to IBus.
            let current = self
                .validated_config
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("IBus config lock poisoned".into()))?;
            let mut instances = self
                .instances
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?;
            let current_generation = self.config_generation.load(Ordering::Acquire);
            if current_generation != generation {
                let mut adapter = adapter
                    .lock()
                    .map_err(|_| zbus::fdo::Error::Failed("IBus engine lock poisoned".into()))?;
                adapter.replace_validated_config(Arc::clone(&current));
            }
            instances.insert(
                path.clone(),
                EngineInstance {
                    adapter: Arc::clone(&adapter),
                    config_generation: AtomicU64::new(current_generation),
                },
            );
        }
        if let Err(error) = connection
            .inner()
            .object_server()
            .at(path.as_str(), engine)
            .await
        {
            let removed = self
                .instances
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?
                .remove(&path)
                .is_some();
            if removed {
                if let Some(telemetry) = &self.telemetry {
                    refresh_context_diagnostics(self.instances.as_ref(), telemetry);
                }
            }
            return Err(zbus::fdo::Error::ZBus(error));
        }
        if let Some(telemetry) = &self.telemetry {
            refresh_context_diagnostics(self.instances.as_ref(), telemetry);
        }
        Ok(path)
    }
}

fn config_generation_mismatch_count(
    instances: &HashMap<OwnedObjectPath, EngineInstance>,
    generation: u64,
) -> usize {
    instances
        .values()
        .filter(|instance| instance.config_generation.load(Ordering::Acquire) != generation)
        .count()
}

fn refresh_context_diagnostics(
    instances: &Mutex<HashMap<OwnedObjectPath, EngineInstance>>,
    telemetry: &IbusTelemetry,
) {
    let generation = telemetry.snapshot().config_generation_requested;
    if let Ok(instances) = instances.lock() {
        telemetry.set_context_diagnostics(
            generation,
            instances.len(),
            config_generation_mismatch_count(&instances, generation),
        );
    }
}

/// Install a validated generation as one handoff. The caller already holds
/// the factory snapshot mutex. The instance map blocks context creation and
/// destruction while all adapter locks are first acquired. If any lock is
/// poisoned, no adapter or factory snapshot is changed.
fn publish_config_generation(
    current: &mut Arc<ValidatedConfig>,
    current_generation: &AtomicU64,
    instances: &Instances,
    validated: Arc<ValidatedConfig>,
    generation: u64,
) -> Result<(), (&'static str, usize, String)> {
    let instances = instances.lock().map_err(|_| {
        (
            "WX-IBUS-RELOAD-001",
            0,
            "IBus engine-instance registry lock is poisoned".to_owned(),
        )
    })?;
    let mut adapters = Vec::with_capacity(instances.len());
    for instance in instances.values() {
        match instance.adapter.lock() {
            Ok(adapter) => adapters.push(adapter),
            Err(_) => {
                let mismatches = instances
                    .values()
                    .filter(|instance| {
                        instance.config_generation.load(Ordering::Acquire) != generation
                    })
                    .count();
                return Err((
                    "WX-IBUS-RELOAD-001",
                    mismatches,
                    "an IBus engine-context lock is poisoned".to_owned(),
                ));
            }
        }
    }

    for (instance, adapter) in instances.values().zip(adapters.iter_mut()) {
        adapter.replace_validated_config(Arc::clone(&validated));
        instance
            .config_generation
            .store(generation, Ordering::Release);
    }
    *current = validated;
    current_generation.store(generation, Ordering::Release);
    Ok(())
}

fn validate_config_for_policy(
    config: &Config,
    policy: &OrganizationPolicy,
) -> Result<Arc<ValidatedConfig>, wayexpand_core::ConfigError> {
    let mut effective = config.clone();
    effective.apply_administrator_policy(policy)?;
    Ok(Arc::new(effective.validate_snapshot()?))
}

fn engine_path(id: u64) -> OwnedObjectPath {
    OwnedObjectPath::try_from(format!("{ENGINE_PATH}/{id}"))
        .expect("generated IBus engine path must be valid")
}

struct EngineObject {
    adapter: Arc<Mutex<IbusEngineAdapter>>,
    connection: Arc<Mutex<Option<Connection>>>,
    path: OwnedObjectPath,
    instances: Arc<Mutex<HashMap<OwnedObjectPath, EngineInstance>>>,
    telemetry: Option<Arc<IbusTelemetry>>,
    gnome_focus: Arc<Mutex<Option<WindowContext>>>,
}

impl EngineObject {
    fn connection(&self) -> zbus::fdo::Result<zbus::Connection> {
        Ok(self
            .connection
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus connection lock poisoned".into()))?
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("IBus connection unavailable".into()))?
            .into_inner())
    }

    async fn emit_actions(&self, actions: &[IbusAction]) -> zbus::fdo::Result<()> {
        if actions.is_empty() {
            return Ok(());
        }
        let connection = self.connection()?;
        for action in actions {
            let result = match action {
                IbusAction::DeleteSurroundingText { nchars } => {
                    connection
                        .emit_signal(
                            None::<&str>,
                            self.path.as_str(),
                            "org.freedesktop.IBus.Engine",
                            "DeleteSurroundingText",
                            &(-(*nchars as i32), *nchars),
                        )
                        .await
                }
                IbusAction::CommitText(text) => {
                    connection
                        .emit_signal(
                            None::<&str>,
                            self.path.as_str(),
                            "org.freedesktop.IBus.Engine",
                            "CommitText",
                            &(Value::from(ibus_text_value(text.as_str())),),
                        )
                        .await
                }
            };
            result.map_err(zbus::fdo::Error::ZBus)?;
        }
        Ok(())
    }

    /// Ask the client to start reporting surrounding text. IBus clients only
    /// send `SetSurroundingText` after an engine has requested it, and
    /// replacements are refused without it.
    async fn require_surrounding_text(&self) {
        if let Ok(connection) = self.connection() {
            let _ = connection
                .emit_signal(
                    None::<&str>,
                    self.path.as_str(),
                    "org.freedesktop.IBus.Engine",
                    "RequireSurroundingText",
                    &(),
                )
                .await;
        }
    }
}

/// Emit replacement actions as IBus engine signals from a plain thread (the
/// completion pump). D-Bus handlers use [`EngineObject::emit_actions`].
fn emit_actions(
    connection: &Connection,
    path: &OwnedObjectPath,
    actions: &[IbusAction],
) -> zbus::fdo::Result<()> {
    for action in actions {
        let result = match action {
            IbusAction::DeleteSurroundingText { nchars } => connection.emit_signal(
                None::<&str>,
                path.as_str(),
                "org.freedesktop.IBus.Engine",
                "DeleteSurroundingText",
                &(-(*nchars as i32), *nchars),
            ),
            IbusAction::CommitText(text) => connection.emit_signal(
                None::<&str>,
                path.as_str(),
                "org.freedesktop.IBus.Engine",
                "CommitText",
                &(Value::from(ibus_text_value(text.as_str())),),
            ),
        };
        result.map_err(zbus::fdo::Error::ZBus)?;
    }
    Ok(())
}

/// Deliver asynchronous command results. Expansion workers wake this loop
/// with the owning engine's path; it blocks on the channel rather than
/// polling. Each result is validated and emitted under the engine's adapter
/// lock (the same lock `ProcessKeyEvent` holds), and recorded as applied only
/// after its signals were emitted; that is not a client-document receipt.
fn run_completion_pump(
    wakeups: mpsc::Receiver<OwnedObjectPath>,
    instances: Instances,
    connection: Arc<Mutex<Option<Connection>>>,
) {
    for path in wakeups {
        let adapter = match instances.lock() {
            Ok(instances) => instances
                .get(&path)
                .map(|instance| Arc::clone(&instance.adapter)),
            Err(_) => return,
        };
        // A destroyed engine's late completion has nowhere to go.
        let Some(adapter) = adapter else { continue };
        let connection = connection.lock().ok().and_then(|slot| slot.clone());
        let Ok(mut adapter) = adapter.lock() else {
            continue;
        };
        let Some(connection) = connection else {
            // Without a bus nothing can be emitted; drop the results so the
            // engine does not later apply them at a moved cursor.
            let _ = adapter.drain_completed_commands_with(|_| {
                Err(zbus::fdo::Error::Failed(
                    "IBus connection unavailable".into(),
                ))
            });
            continue;
        };
        if let Err(error) = adapter
            .drain_completed_commands_with(|actions| emit_actions(&connection, &path, actions))
        {
            warn!(error = %error, "IBus could not emit an asynchronous expansion; engine reset");
        }
    }
}

/// Extract the string from a serialized IBusText variant.
fn ibus_text_string(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Value(inner) => ibus_text_string(inner),
        Value::Structure(structure) => match structure.fields() {
            [Value::Str(name), _, Value::Str(text), ..] if name.as_str() == "IBusText" => {
                Some(text.to_string())
            }
            _ => None,
        },
        _ => None,
    }
}

/// `spawn = false` handles calls to engine objects one at a time, in arrival
/// order, so key events and the signals they emit cannot be reordered. The
/// adapter lock is never held across an `.await`.
#[interface(name = "org.freedesktop.IBus.Engine", spawn = false)]
impl EngineObject {
    async fn process_key_event(&self, keyval: u32, keycode: u32, state: u32) -> bool {
        let result = match self.adapter.lock() {
            Ok(mut adapter) => {
                let focus = self.gnome_focus.lock().ok().and_then(|focus| focus.clone());
                adapter.update_gnome_focus(focus.as_ref());
                adapter.process_key_event(keyval, keycode, state)
            }
            Err(_) => return false,
        };
        if self.emit_actions(&result.actions).await.is_err() {
            // IBus emits delete and commit as separate signals. An error may
            // therefore leave a partial edit; retire this engine until a new
            // focus lifecycle instead of allowing more expansions in an
            // uncertain document state.
            if let Ok(mut adapter) = self.adapter.lock() {
                adapter.output_failed();
            }
            return false;
        }
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.output_succeeded();
        }
        result.handled
    }

    async fn focus_in(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_in_for_context(Some(self.path.as_str()));
            let focus = self.gnome_focus.lock().ok().and_then(|focus| focus.clone());
            adapter.bind_gnome_focus(focus.as_ref());
        }
        self.require_surrounding_text().await;
    }
    fn focus_out(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_out();
        }
    }
    fn reset(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.reset();
        }
    }
    async fn enable(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.enable();
        }
        self.require_surrounding_text().await;
    }
    fn disable(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_out();
        }
    }
    /// IBus calls Destroy when the input context releases this engine. Remove
    /// both the retained adapter and its D-Bus object so context churn cannot
    /// grow the service forever.
    ///
    /// Async so the object-server removal is awaited on the dispatcher
    /// rather than blocking it: removal takes the object tree's write lock,
    /// which a blocking call from inside a handler could wait on forever.
    async fn destroy(&self) -> zbus::fdo::Result<()> {
        let removed = {
            let mut instances = self
                .instances
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?;
            instances.remove(&self.path).is_some()
        };
        if !removed {
            return Ok(());
        }
        if let Some(telemetry) = &self.telemetry {
            refresh_context_diagnostics(self.instances.as_ref(), telemetry);
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus connection lock poisoned".into()))?
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("IBus connection unavailable".into()))?;
        connection
            .inner()
            .object_server()
            .remove::<EngineObject, _>(self.path.as_str())
            .await
            .map_err(zbus::fdo::Error::ZBus)?;
        Ok(())
    }
    fn set_cursor_location(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
    fn set_capabilities(&self, caps: u32) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.set_capabilities(caps);
        }
    }
    /// Capture is enabled only by a known non-sensitive content type; see
    /// [`crate::content_type_is_sensitive`].
    ///
    /// IBus delivers the content type as the write-only `ContentType`
    /// property, set through `org.freedesktop.DBus.Properties.Set`; the
    /// engine interface has no `SetContentType` method. A method here is
    /// never called, which leaves capture disabled in every field.
    #[zbus(property)]
    fn set_content_type(&self, content_type: (u32, u32)) {
        let (purpose, hints) = content_type;
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.set_content_type(purpose, hints);
        }
    }
    fn set_surrounding_text(&self, text: OwnedValue, cursor_pos: u32, anchor_pos: u32) {
        let Ok(mut adapter) = self.adapter.lock() else {
            return;
        };
        match ibus_text_string(&text) {
            Some(text) => adapter.set_surrounding_text(&text, cursor_pos, anchor_pos),
            // Unreadable text must not leave an older model in place.
            None => adapter.clear_surrounding_text(),
        }
    }
}

/// Build and run the IBus engine process. The process owns a private bus name,
/// registers a factory, and creates one isolated engine object per request
/// while zbus dispatches method calls on its internal async-io executor.
/// Direct executable expansions are disabled in this non-hardened session
/// service. Managed Action Broker expansions remain available when the broker
/// is configured and healthy; they fail closed when it is unavailable.
pub fn run_service(config_path: Option<std::path::PathBuf>) -> Result<(), IbusServiceError> {
    let path = config_path.unwrap_or_else(default_config_path);
    let store = ConfigStore::load(&path)?;
    let policy =
        Arc::new(wayexpand_core::load_organization_policy().map_err(IbusServiceError::Policy)?);
    let initial_snapshot = store.snapshot();
    let initial_status = store.status();
    info!(
        config_state = initial_status.state,
        config_generation = initial_snapshot.generation,
        "IBus configuration loaded"
    );
    let connection_slot = Arc::new(Mutex::new(None));
    let telemetry = Arc::new(IbusTelemetry::new());
    let initial_validated = validate_config_for_policy(&initial_snapshot.config, &policy)?;
    telemetry.config_generation_published(initial_snapshot.generation, 0);
    start_status_writer(Arc::clone(&telemetry))?;
    let factory_config = Arc::new(Mutex::new(initial_validated));
    let config_generation = Arc::new(AtomicU64::new(initial_snapshot.generation));
    let instances: Instances = Arc::new(Mutex::new(HashMap::new()));
    let gnome_focus = Arc::new(Mutex::new(None));
    if env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("gnome")
    {
        spawn_gnome_focus_tracker(Arc::clone(&gnome_focus))?;
    }
    let (completion_sender, completion_receiver) = mpsc::channel();
    let factory = Factory {
        connection: Arc::clone(&connection_slot),
        validated_config: Arc::clone(&factory_config),
        config_generation: Arc::clone(&config_generation),
        policy: Arc::clone(&policy),
        instances: Arc::clone(&instances),
        next_id: AtomicU64::new(1),
        completions: completion_sender,
        telemetry: Some(Arc::clone(&telemetry)),
        gnome_focus,
    };
    let pump_instances = Arc::clone(&instances);
    let pump_connection = Arc::clone(&connection_slot);
    std::thread::Builder::new()
        .name("wayexpand-ibus-completions".into())
        .spawn(move || run_completion_pump(completion_receiver, pump_instances, pump_connection))
        .map_err(|error| IbusServiceError::Thread(error.to_string()))?;
    let reload_receiver = store.subscribe();
    let reload_store = Arc::clone(&store);
    let reload_instances = Arc::clone(&instances);
    let reload_policy = Arc::clone(&policy);
    let reload_validated_config = Arc::clone(&factory_config);
    let reload_config_generation = Arc::clone(&config_generation);
    let reload_telemetry = Arc::clone(&telemetry);
    let config_watch_path = path;
    std::thread::Builder::new()
        .name("wayexpand-ibus-config".into())
        .spawn(move || {
            let (watch_sender, watch_receiver) = mpsc::channel();
            let mut watcher = notify::recommended_watcher(move |event| {
                let _ = watch_sender.send(event);
            })
            .ok();
            if let Some(active_watcher) = watcher.as_mut() {
                let watch_path = config_watch_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                if let Err(error) = active_watcher.watch(watch_path, RecursiveMode::NonRecursive) {
                    warn!(
                        error = %error,
                        path = %watch_path.display(),
                        "IBus configuration watch unavailable; using fallback polling"
                    );
                    watcher = None;
                }
            } else {
                warn!("IBus configuration watch unavailable; using fallback polling");
            }
            let mut reload_error = None;
            loop {
                if watcher.is_some() {
                    let _ = watch_receiver.recv_timeout(Duration::from_secs(30));
                } else {
                    std::thread::sleep(Duration::from_millis(250));
                }
                match reload_store.reload_if_changed() {
                    Ok(true) => {
                        let status = reload_store.status();
                        if reload_error.take().is_some() {
                            info!(
                                config_state = status.state,
                                config_generation = status.generation,
                                "IBus configuration reload recovered"
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let summary = error.safe_summary();
                        if reload_error.as_deref() != Some(summary.as_str()) {
                            let status = reload_store.status();
                            warn!(
                                config_state = status.state,
                                config_error = %summary,
                                config_generation = status.generation,
                                "IBus configuration reload rejected; keeping previous configuration"
                            );
                            reload_error = Some(summary);
                        }
                    }
                }
                while reload_receiver.try_recv().is_ok() {
                    let snapshot = reload_store.snapshot();
                    let validated = match validate_config_for_policy(
                        &snapshot.config,
                        &reload_policy,
                    ) {
                        Ok(validated) => validated,
                        Err(error) => {
                            warn!(
                                config_generation = snapshot.generation,
                                config_error = %error.safe_summary(),
                                "IBus configuration generation rejected after administrator policy"
                            );
                            continue;
                        }
                    };
                    if let Ok(mut current) = reload_validated_config.lock() {
                        let publish = publish_config_generation(
                            &mut current,
                            &reload_config_generation,
                            &reload_instances,
                            Arc::clone(&validated),
                            snapshot.generation,
                        );
                        match publish {
                            Ok(()) => {
                                reload_telemetry
                                    .config_generation_published(snapshot.generation, 0);
                                reload_error = None;
                            }
                            Err((error_id, mismatches, detail)) => {
                                let mismatches = if mismatches == 0 {
                                    reload_telemetry.snapshot().engines as usize
                                } else {
                                    mismatches
                                };
                                reload_telemetry.config_generation_failed(
                                    snapshot.generation,
                                    error_id,
                                    mismatches,
                                );
                                if reload_error.as_deref() != Some(error_id) {
                                    warn!(
                                        problem_id = error_id,
                                        config_generation = snapshot.generation,
                                        context_mismatches = mismatches,
                                        error = detail,
                                        "IBus configuration generation handoff failed; retaining previous published generation"
                                    );
                                }
                                reload_error = Some(error_id.to_owned());
                            }
                        }
                    } else {
                        let error_id = "WX-IBUS-RELOAD-001";
                        let mismatches = reload_telemetry.snapshot().engines as usize;
                        reload_telemetry.config_generation_failed(
                            snapshot.generation,
                            error_id,
                            mismatches,
                        );
                        warn!(
                            problem_id = error_id,
                            config_generation = snapshot.generation,
                            context_mismatches = mismatches,
                            "IBus factory config lock poisoned; retaining previous generation"
                        );
                        reload_error = Some(error_id.to_owned());
                    }
                }
            }
        })
        .map_err(|error| IbusServiceError::Thread(error.to_string()))?;
    // IBus engines must connect to IBus' private bus, not the ordinary
    // desktop session bus. The daemon supplies IBUS_ADDRESS when launching
    // an engine; the command fallback also supports manual startup.
    let builder = match env::var("IBUS_ADDRESS") {
        Ok(address) => Builder::address(address.as_str())?,
        Err(_) => Builder::ibus()?,
    };
    let connection = builder
        .name(BUS_NAME)?
        .serve_at(FACTORY_PATH, factory)?
        .build()?;
    *connection_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
    loop {
        std::thread::park();
    }
}

/// Publish engine activity for `wayexpand doctor` and `wayexpand ibus`.
/// Writes are coalesced: at most a few per second however fast keys arrive.
fn start_status_writer(telemetry: Arc<IbusTelemetry>) -> Result<(), IbusServiceError> {
    let Some(path) = telemetry::status_path() else {
        warn!("XDG_RUNTIME_DIR is unset; IBus activity diagnostics are unavailable");
        return Ok(());
    };
    let initial = telemetry.snapshot();
    std::thread::Builder::new()
        .name("wayexpand-ibus-status".into())
        .spawn(move || {
            let mut reported_error = false;
            let mut publish =
                |status: &telemetry::IbusStatus| match telemetry::write_status(&path, status) {
                    Ok(()) => reported_error = false,
                    Err(error) if !reported_error => {
                        warn!(error = %error, "could not publish IBus activity diagnostics");
                        reported_error = true;
                    }
                    Err(_) => {}
                };
            publish(&initial);
            loop {
                if let Some(status) = telemetry.wait_for_change(Duration::from_secs(30)) {
                    publish(&status);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        })
        .map_err(|error| IbusServiceError::Thread(error.to_string()))?;
    Ok(())
}

/// Build the serialized IBusText object with an empty attribute list. IBus
/// represents text as a serialized object inside a D-Bus variant.
fn ibus_text_value(text: &str) -> zbus::zvariant::Structure<'static> {
    let empty: HashMap<String, Value<'static>> = HashMap::new();
    let attrs = StructureBuilder::new()
        .add_field("IBusAttrList")
        .add_field(empty.clone())
        .add_field(Vec::<Value<'static>>::new())
        .build()
        .expect("valid IBus attribute structure");
    StructureBuilder::new()
        .add_field("IBusText")
        .add_field(empty)
        .add_field(text.to_owned())
        .add_field(Value::from(attrs))
        .build()
        .expect("valid IBus text structure")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::ExpansionEngine;

    #[test]
    fn bus_name_matches_the_ibus_component_name() {
        let manifest = include_str!("../../../desktop/wayexpand-ibus.xml");
        let component_name = manifest
            .split_once("<name>")
            .and_then(|(_, rest)| rest.split_once("</name>"))
            .map(|(name, _)| name.trim())
            .expect("component manifest declares a name");
        assert_eq!(BUS_NAME, component_name);
    }
    use wayexpand_core::{Config, ReplacementGuarantee};
    use zbus::zvariant::DynamicType;

    #[test]
    fn ibus_text_has_the_serialized_object_signature() {
        let text = ibus_text_value("hello");
        assert_eq!(text.signature().to_string(), "(sa{sv}sv)");
        let signal_body = (Value::from(text),);
        assert_eq!(signal_body.signature().to_string(), "(v)");
    }

    #[test]
    fn delete_signal_uses_signed_offset_and_unsigned_character_count() {
        let body = (-4_i32, 4_u32);
        assert_eq!(body.signature().to_string(), "(iu)");
    }

    #[test]
    fn factory_allocates_distinct_engine_paths() {
        let first = engine_path(1);
        let second = engine_path(2);
        assert_eq!(first.as_str(), "/org/freedesktop/IBus/Engine/WayExpand/1");
        assert_eq!(second.as_str(), "/org/freedesktop/IBus/Engine/WayExpand/2");
        assert_ne!(first, second);
    }

    #[test]
    fn factory_rejects_unknown_engine_names() {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":x\"\nreplacement = \"x\"\n").unwrap();
        let policy = Arc::new(OrganizationPolicy::default());
        let factory = Factory {
            connection: Arc::new(Mutex::new(None)),
            validated_config: Arc::new(Mutex::new(
                validate_config_for_policy(&config, &policy).unwrap(),
            )),
            config_generation: Arc::new(AtomicU64::new(1)),
            policy,
            instances: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            completions: mpsc::channel().0,
            telemetry: None,
            gnome_focus: Arc::new(Mutex::new(None)),
        };
        assert!(matches!(
            zbus::block_on(factory.create_engine("not-wayexpand")),
            Err(zbus::fdo::Error::InvalidArgs(_))
        ));
    }

    #[test]
    fn destroy_removes_instance_before_connection_teardown() {
        let path = engine_path(7);
        let instances = Arc::new(Mutex::new(HashMap::new()));
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":x\"\nreplacement = \"x\"\n").unwrap();
        instances.lock().unwrap().insert(
            path.clone(),
            EngineInstance {
                adapter: Arc::new(Mutex::new(IbusEngineAdapter::new(
                    ExpansionEngine::new(config).unwrap(),
                ))),
                config_generation: AtomicU64::new(1),
            },
        );
        let engine = EngineObject {
            adapter: Arc::new(Mutex::new(IbusEngineAdapter::new(
                ExpansionEngine::new(
                    toml::from_str("[[expansion]]\ntrigger = \":x\"\nreplacement = \"x\"\n")
                        .unwrap(),
                )
                .unwrap(),
            ))),
            connection: Arc::new(Mutex::new(None)),
            path,
            instances: Arc::clone(&instances),
            telemetry: None,
            gnome_focus: Arc::new(Mutex::new(None)),
        };

        assert!(zbus::block_on(engine.destroy()).is_err());
        assert!(instances.lock().unwrap().is_empty());
    }

    #[test]
    fn password_and_pin_content_types_disable_expansion() {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":x\"\nreplacement = \"expanded\"\n")
                .unwrap();
        let engine = EngineObject {
            adapter: Arc::new(Mutex::new(IbusEngineAdapter::new(
                ExpansionEngine::new(config).unwrap(),
            ))),
            connection: Arc::new(Mutex::new(None)),
            path: engine_path(1),
            instances: Arc::new(Mutex::new(HashMap::new())),
            telemetry: None,
            gnome_focus: Arc::new(Mutex::new(None)),
        };
        engine.set_content_type((8, 0));
        assert!(!zbus::block_on(engine.process_key_event('a' as u32, 0, 0)));
        engine.set_content_type((0, 0));
        assert!(!engine.adapter.lock().unwrap().engine().is_sensitive_focus());
    }

    #[test]
    fn synchronous_signal_failure_disables_the_engine_until_refocus() {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = "sig"
replacement = "signature"
"#,
        )
        .unwrap();
        let adapter = Arc::new(Mutex::new(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        )));
        {
            let mut adapter = adapter.lock().unwrap();
            adapter
                .engine_mut()
                .set_compatibility_mode(wayexpand_core::CompatibilityMode::TrackedFocus);
            adapter.focus_in();
            adapter.set_capabilities(crate::IBUS_CAP_SURROUNDING_TEXT);
            adapter.set_content_type(0, 0);
            adapter.set_surrounding_text("", 0, 0);
        }
        let engine = EngineObject {
            adapter: Arc::clone(&adapter),
            connection: Arc::new(Mutex::new(None)),
            path: engine_path(2),
            instances: Arc::new(Mutex::new(HashMap::new())),
            telemetry: None,
            gnome_focus: Arc::new(Mutex::new(None)),
        };

        // Ordinary key output fails because there is no live D-Bus transport.
        // The service must retire this engine rather than processing a
        // partial trigger if more keys arrive in the same focus context.
        assert!(!zbus::block_on(engine.process_key_event('s' as u32, 0, 0)));
        zbus::block_on(engine.enable());
        assert!(
            !adapter
                .lock()
                .unwrap()
                .process_key_event('i' as u32, 0, 0)
                .handled
        );
    }

    /// Broker-backed tests share the process-wide broker socket variable, so
    /// they must not overlap even when tests run on many threads.
    static BROKER_LOCK: Mutex<()> = Mutex::new(());

    const BROKER_CONFIG: &str =
        "[settings]\ncompatibility_mode = \"tracked-focus\"\nundo_chord = \"Ctrl+Z\"\n\
         [[expansion]]\ntrigger = \":ok\"\nreplacement = \"\"\n\
         [expansion.command]\naction = \"status\"\ntimeout_ms = 5000\n";

    /// Serve exactly one broker request on `socket`, answering with `stdout`
    /// after `delay`. Returns the action id the engine asked for.
    fn fake_broker(
        socket: std::path::PathBuf,
        stdout: &'static str,
        delay: Duration,
    ) -> std::thread::JoinHandle<Result<String, String>> {
        use std::io::{BufRead, Write};
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let (stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Err("no broker request arrived within 10 seconds".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(format!("broker accept failed: {error}")),
                }
            };
            let mut request = String::new();
            std::io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?)
                .read_line(&mut request)
                .map_err(|error| error.to_string())?;
            std::thread::sleep(delay);
            let response = serde_json::json!({ "Success": {
                "exit_code": 0, "stdout": stdout, "stderr": "",
                "stdout_truncated": false, "stderr_truncated": false, "duration_ms": 1,
            }});
            let mut stream = stream;
            // Destroy/focus tests intentionally close the client while the
            // broker is working; a broken pipe is expected there and is not
            // evidence that the request failed to reach the fake broker.
            let _ = writeln!(stream, "{response}");
            let request: serde_json::Value =
                serde_json::from_str(&request).map_err(|error| error.to_string())?;
            Ok(request["action_id"].as_str().unwrap_or_default().to_owned())
        })
    }

    /// A broker answering one request, with the socket variable pointing at
    /// it for the lifetime of the value.
    struct BrokerFixture {
        _guard: std::sync::MutexGuard<'static, ()>,
        root: std::path::PathBuf,
        previous_socket: Option<std::ffi::OsString>,
        broker: Option<std::thread::JoinHandle<Result<String, String>>>,
    }

    impl BrokerFixture {
        fn start(name: &str, stdout: &'static str, delay: Duration) -> Self {
            let guard = BROKER_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let root =
                std::env::temp_dir().join(format!("wayexpand-ibus-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let socket = root.join("broker.sock");
            let previous_socket = std::env::var_os("WAYEXPAND_ACTION_BROKER_SOCKET");
            std::env::set_var("WAYEXPAND_ACTION_BROKER_SOCKET", &socket);
            let broker = Some(fake_broker(socket, stdout, delay));
            Self {
                _guard: guard,
                root,
                previous_socket,
                broker,
            }
        }

        /// Wait until the broker has answered; returns the requested action.
        fn answered(&mut self) -> String {
            self.broker
                .take()
                .unwrap()
                .join()
                .expect("fake broker thread panicked")
                .expect("fake broker did not receive and answer an action")
        }
    }

    impl Drop for BrokerFixture {
        fn drop(&mut self) {
            match self.previous_socket.take() {
                Some(value) => std::env::set_var("WAYEXPAND_ACTION_BROKER_SOCKET", value),
                None => std::env::remove_var("WAYEXPAND_ACTION_BROKER_SOCKET"),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The real factory, completion pump, and engine objects behind a
    /// peer-to-peer D-Bus connection, with a client that records every
    /// `CommitText` it receives.
    struct Harness {
        client: Connection,
        engine: OwnedObjectPath,
        commits: mpsc::Receiver<String>,
        deletions: mpsc::Receiver<u32>,
        instances: Instances,
        telemetry: Arc<IbusTelemetry>,
        connection_slot: Arc<Mutex<Option<Connection>>>,
        completion_pump: Option<std::thread::JoinHandle<()>>,
        message_reader: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            // The test server owns Factory, and Factory owns this slot. Clear
            // the connection to break that test-only Arc cycle and let its
            // D-Bus objects drop so the pump and client reader can terminate.
            if let Ok(mut slot) = self.connection_slot.lock() {
                slot.take();
            }
            if let Ok(mut instances) = self.instances.lock() {
                instances.clear();
            }
            if let Some(pump) = self.completion_pump.take() {
                let _ = pump.join();
            }
            if let Some(reader) = self.message_reader.take() {
                let _ = reader.join();
            }
        }
    }

    impl Harness {
        fn start(config: &str) -> Self {
            let config: Config = toml::from_str(config).unwrap();
            let policy = Arc::new(OrganizationPolicy::default());
            let connection_slot = Arc::new(Mutex::new(None));
            let instances: Instances = Arc::new(Mutex::new(HashMap::new()));
            let telemetry = Arc::new(IbusTelemetry::new());
            let (completion_sender, completion_receiver) = mpsc::channel();
            let factory = Factory {
                connection: Arc::clone(&connection_slot),
                validated_config: Arc::new(Mutex::new(
                    validate_config_for_policy(&config, &policy).unwrap(),
                )),
                config_generation: Arc::new(AtomicU64::new(1)),
                policy,
                instances: Arc::clone(&instances),
                next_id: AtomicU64::new(1),
                completions: completion_sender,
                telemetry: Some(Arc::clone(&telemetry)),
                gnome_focus: Arc::new(Mutex::new(None)),
            };
            let (server_stream, client_stream) = std::os::unix::net::UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                Builder::async_io_unix_stream(server_stream)
                    .server(zbus::Guid::generate())
                    .unwrap()
                    .p2p()
                    .serve_at(FACTORY_PATH, factory)
                    .unwrap()
                    .build()
                    .unwrap()
            });
            let client = Builder::async_io_unix_stream(client_stream)
                .p2p()
                .build()
                .unwrap();
            let server = server.join().unwrap();
            *connection_slot.lock().unwrap() = Some(server);
            let completion_pump = {
                let instances = Arc::clone(&instances);
                let connection = Arc::clone(&connection_slot);
                std::thread::spawn(move || {
                    run_completion_pump(completion_receiver, instances, connection)
                })
            };
            let (commit_sender, commits) = mpsc::channel();
            let (deletion_sender, deletions) = mpsc::channel();
            let messages = zbus::blocking::MessageIterator::from(&client);
            let message_reader = std::thread::spawn(move || {
                for message in messages.flatten() {
                    match message.header().member().map(|member| member.as_str()) {
                        Some("CommitText") => {
                            let (text,): (OwnedValue,) = message.body().deserialize().unwrap();
                            let _ = commit_sender.send(ibus_text_string(&text).unwrap());
                        }
                        Some("DeleteSurroundingText") => {
                            let (_, nchars): (i32, u32) = message.body().deserialize().unwrap();
                            let _ = deletion_sender.send(nchars);
                        }
                        _ => {}
                    }
                }
            });
            let engine: OwnedObjectPath = client
                .call_method(
                    None::<&str>,
                    FACTORY_PATH,
                    Some("org.freedesktop.IBus.Factory"),
                    "CreateEngine",
                    &("wayexpand",),
                )
                .unwrap()
                .body()
                .deserialize()
                .unwrap();
            Self {
                client,
                engine,
                commits,
                deletions,
                instances,
                telemetry,
                connection_slot,
                completion_pump: Some(completion_pump),
                message_reader: Some(message_reader),
            }
        }

        fn call<B>(&self, method: &str, body: &B) -> zbus::Message
        where
            B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        {
            self.client
                .call_method(
                    None::<&str>,
                    self.engine.as_str(),
                    Some("org.freedesktop.IBus.Engine"),
                    method,
                    body,
                )
                .unwrap()
        }

        /// Set the content type the way ibus-daemon does: through the
        /// engine's write-only `ContentType` property.
        fn set_content_type(&self, purpose: u32, hints: u32) -> zbus::Message {
            self.client
                .call_method(
                    None::<&str>,
                    self.engine.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Set",
                    &(
                        "org.freedesktop.IBus.Engine",
                        "ContentType",
                        Value::from((purpose, hints)),
                    ),
                )
                .unwrap()
        }

        fn introspect(&self) -> String {
            self.client
                .call_method(
                    None::<&str>,
                    self.engine.as_str(),
                    Some("org.freedesktop.DBus.Introspectable"),
                    "Introspect",
                    &(),
                )
                .unwrap()
                .body()
                .deserialize()
                .unwrap()
        }

        /// The sequence ibus-daemon sends when an ordinary text field gains
        /// focus.
        fn focus_text_field(&self) {
            self.call("FocusIn", &());
            self.call("Enable", &());
            self.call("SetCapabilities", &(crate::IBUS_CAP_SURROUNDING_TEXT,));
            self.set_content_type(0, 0);
        }

        /// Report `before` as the text before the cursor, then press `key`.
        fn key(&self, key: char, before: &str) -> bool {
            let cursor = before.chars().count() as u32;
            self.call(
                "SetSurroundingText",
                &(Value::from(ibus_text_value(before)), cursor, cursor),
            );
            self.call("ProcessKeyEvent", &(key as u32, 0_u32, 0_u32))
                .body()
                .deserialize()
                .unwrap()
        }

        /// Type `:ok`: `:` passes through, `o` and `k` are committed by the
        /// engine, and `k` queues the broker action.
        fn type_trigger(&self) {
            let handled: Vec<bool> = [(':', ""), ('o', ":"), ('k', ":o")]
                .into_iter()
                .map(|(key, before)| self.key(key, before))
                .collect();
            assert_eq!(handled, [false, true, true]);
        }

        /// Every commit received until `quiet` passes with nothing new.
        fn commits(&self, quiet: Duration) -> Vec<String> {
            let mut commits = Vec::new();
            while let Ok(text) = self.commits.recv_timeout(quiet) {
                commits.push(text);
            }
            commits
        }

        /// Wait for the correlated asynchronous result instead of treating a
        /// short quiet period after the trigger commits as proof that the
        /// completion pump has finished. Release-mode workspace tests can
        /// schedule the action worker later under load.
        fn commits_through(&self, expected: &str, timeout: Duration) -> Vec<String> {
            let deadline = std::time::Instant::now() + timeout;
            let mut commits = Vec::new();
            while std::time::Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match self.commits.recv_timeout(remaining) {
                    Ok(text) => {
                        let matched = text == expected;
                        commits.push(text);
                        if matched {
                            commits.extend(self.commits.try_iter());
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            commits
        }
    }

    /// libibus declares `<property name='ContentType' type='(uu)'
    /// access='write' />` on `org.freedesktop.IBus.Engine`, and ibus-daemon
    /// sets it with `Properties.Set`. Pin that shape so capture cannot
    /// silently stay disabled again.
    #[test]
    fn content_type_is_the_write_only_property_ibus_daemon_sets() {
        let harness = Harness::start(BROKER_CONFIG);
        let xml = harness.introspect();
        assert!(
            xml.contains(r#"<property name="ContentType" type="(uu)" access="write""#),
            "engine introspection lacks IBus' ContentType property:\n{xml}"
        );
        assert!(!xml.contains(r#"name="SetContentType""#));
        harness.set_content_type(0, 0);
    }

    #[test]
    fn focus_in_binds_forms_to_the_factory_input_context_path() {
        let harness = Harness::start(
            "[settings]\ncompatibility_mode = 'tracked-focus'\n\n[[expansion]]\ntrigger = ':incident'\nreplacement = 'Incident {{field:hostname}}'\n",
        );
        assert_eq!(
            harness.instances.lock().unwrap()[&harness.engine]
                .adapter
                .lock()
                .unwrap()
                .engine()
                .compatibility_mode(),
            wayexpand_core::CompatibilityMode::TrackedFocus
        );
        harness.focus_text_field();
        let instances = harness.instances.lock().unwrap();
        let instance = instances.get(&harness.engine).expect("created IBus engine");
        let adapter = instance.adapter.lock().unwrap();
        let context = adapter
            .engine()
            .current_window()
            .expect("focused input context identity");
        assert_eq!(
            context.instance_id.as_deref(),
            Some(format!("ibus-context:{}", harness.engine.as_str()).as_str())
        );
        assert_eq!(
            adapter.engine().form_replacement_guarantee(),
            ReplacementGuarantee::VerifiedSurroundingText
        );
    }

    #[cfg(unix)]
    #[test]
    fn form_completion_pump_delivers_after_the_origin_context_refocuses() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = crate::FORM_HELPER_ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "wayexpand-ibus-form-pump-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let helper = root.join("form-helper");
        let release = root.join("release");
        let script = format!(
            "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.01; done\nprintf '%s' '{{\"field:hostname\":\"node-a\"}}'\n",
            release.display()
        );
        std::fs::write(&helper, script).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();

        struct RestoreFormHelper {
            previous: Option<std::ffi::OsString>,
            root: std::path::PathBuf,
        }
        impl Drop for RestoreFormHelper {
            fn drop(&mut self) {
                if let Some(value) = self.previous.take() {
                    std::env::set_var("WAYEXPAND_FORM_HELPER", value);
                } else {
                    std::env::remove_var("WAYEXPAND_FORM_HELPER");
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let _restore_helper = RestoreFormHelper {
            previous: std::env::var_os("WAYEXPAND_FORM_HELPER"),
            root,
        };
        std::env::set_var("WAYEXPAND_FORM_HELPER", &helper);

        let harness = Harness::start(
            "[[expansion]]\ntrigger = ':incident'\nreplacement = 'Incident {{field:hostname}}'\n",
        );
        harness.focus_text_field();
        let mut before = String::new();
        for character in ":incident".chars() {
            harness.key(character, &before);
            before.push(character);
        }
        harness.instances.lock().unwrap()[&harness.engine]
            .adapter
            .lock()
            .unwrap()
            .engine_mut()
            .set_compatibility_mode(wayexpand_core::CompatibilityMode::TrackedFocus);

        // Hold the helper open while its IBus context loses and regains focus.
        // This exercises the real completion pump, not just adapter polling.
        harness.call("FocusOut", &());
        harness.focus_text_field();
        let cursor = before.chars().count() as u32;
        harness.call(
            "SetSurroundingText",
            &(Value::from(ibus_text_value(&before)), cursor, cursor),
        );
        std::fs::write(&release, "ready").unwrap();

        let commits = harness.commits(Duration::from_secs(2));
        assert!(
            commits.iter().any(|text| text == "Incident node-a"),
            "form result was not committed after refocus: {commits:?}"
        );
        assert_eq!(
            harness
                .deletions
                .recv_timeout(Duration::from_millis(100))
                .expect("form replacement must delete the original trigger"),
            ":incident".chars().count() as u32
        );
    }

    #[test]
    fn broker_result_reaches_the_ibus_client_through_the_completion_pump() {
        let mut broker =
            BrokerFixture::start("delivered", "from-broker", Duration::from_millis(150));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        assert_eq!(broker.answered(), "status");
        // Workspace tests start many subprocess-backed cases in parallel;
        // retain a finite bound and include content-free runtime state if the
        // command worker fails to deliver under runner contention.
        let commits = harness.commits_through("from-broker", Duration::from_secs(35));
        let engine_metrics = harness.instances.lock().unwrap()[&harness.engine]
            .adapter
            .lock()
            .unwrap()
            .engine()
            .command_metrics();
        assert_eq!(
            commits,
            ["o", "k", "from-broker"],
            "command metrics: {engine_metrics:?}; IBus telemetry: {:?}",
            harness.telemetry.snapshot()
        );
    }

    #[test]
    fn completion_after_destroy_is_dropped_without_output() {
        let mut broker =
            BrokerFixture::start("destroyed", "from-broker", Duration::from_millis(200));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        harness.call("Destroy", &());
        assert!(harness.instances.lock().unwrap().is_empty());
        broker.answered();
        assert_eq!(harness.commits(Duration::from_millis(500)), ["o", "k"]);
    }

    #[test]
    fn switching_to_a_password_field_discards_an_in_flight_result() {
        let mut broker =
            BrokerFixture::start("to-password", "from-broker", Duration::from_millis(200));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        harness.set_content_type(8, 0);
        broker.answered();
        assert_eq!(harness.commits(Duration::from_millis(500)), ["o", "k"]);
    }

    /// The sequence captured from ibus-daemon 1.5.34 when focus returns to a
    /// field whose content type equals the last one it set: FocusOut,
    /// FocusIn and SetCapabilities, with no ContentType property write.
    #[test]
    fn refocus_to_a_field_of_the_same_type_keeps_capture_enabled() {
        // This test only checks that the trigger remains capturable after the
        // focus lifecycle. Keep it snippet-only: broker fixtures temporarily
        // set a process-wide socket override, and an unrelated command-backed
        // test could otherwise race with their one-request fake broker.
        let harness = Harness::start(
            "[settings]\ncompatibility_mode = \"tracked-focus\"\n\
             [[expansion]]\ntrigger = \":ok\"\nreplacement = \"ok\"\n",
        );
        harness.focus_text_field();
        harness.call("FocusOut", &());
        harness.call("FocusIn", &());
        harness.call("SetCapabilities", &(crate::IBUS_CAP_SURROUNDING_TEXT,));
        harness.type_trigger();
    }

    /// Telemetry explains both outcomes seen on a real desktop: an editor
    /// that reports its text expands, and a terminal-like client that
    /// reports only empty text is refused with a specific reason.
    #[test]
    fn telemetry_distinguishes_expanding_fields_from_terminal_like_fields() {
        let harness = Harness::start(
            "[settings]\ncompatibility_mode = 'tracked-focus'\n\n[[expansion]]\ntrigger = ';ok'\nreplacement = 'done'\n",
        );
        harness.focus_text_field();
        assert!(!harness.key(';', ""));
        assert!(harness.key('o', ";"));
        assert!(harness.key('k', ";o"));
        assert_eq!(
            harness.deletions.recv_timeout(Duration::from_secs(2)),
            Ok(2)
        );
        let editor = harness.telemetry.snapshot().recent_fields.pop().unwrap();
        assert_eq!(editor.expansions, 1);
        assert_eq!(editor.content_type, telemetry::IbusContentTypeState::Normal);
        assert_eq!(
            crate::IbusFieldVerdict::of(&editor),
            crate::IbusFieldVerdict::Expanded
        );

        // A terminal: same content type (IBus does not set it again), one
        // empty surrounding-text report, then keys with no further reports.
        harness.call("FocusOut", &());
        harness.call("FocusIn", &());
        harness.call("SetCapabilities", &(crate::IBUS_CAP_SURROUNDING_TEXT,));
        harness.call(
            "SetSurroundingText",
            &(Value::from(ibus_text_value("")), 0_u32, 0_u32),
        );
        for key in [';', 'o', 'k'] {
            harness.call("ProcessKeyEvent", &(key as u32, 0_u32, 0_u32));
        }
        let status = harness.telemetry.snapshot();
        let terminal = status.recent_fields.last().unwrap();
        assert_eq!(terminal.expansions, 0);
        assert_eq!(terminal.refusals.surrounding_text_empty, 1);
        assert_eq!(
            crate::IbusFieldVerdict::of(terminal),
            crate::IbusFieldVerdict::NoSurroundingText
        );
        assert_eq!(status.totals.expansions, 1);
        assert_eq!(status.engines, 1);
    }

    #[test]
    fn refocus_without_a_content_type_discards_an_in_flight_result() {
        let mut broker = BrokerFixture::start("refocus", "from-broker", Duration::from_millis(200));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        harness.call("FocusOut", &());
        harness.call("FocusIn", &());
        broker.answered();
        assert_eq!(harness.commits(Duration::from_millis(500)), ["o", "k"]);
    }

    #[test]
    fn typing_before_completion_discards_the_stale_result() {
        let mut broker =
            BrokerFixture::start("typing-on", "from-broker", Duration::from_millis(200));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        assert!(harness.key('x', ":ok"));
        broker.answered();
        assert_eq!(harness.commits(Duration::from_millis(500)), ["o", "k", "x"]);
    }

    #[test]
    fn failed_emission_after_delete_resets_without_recording_the_expansion() {
        // Fault injection: the delete was sent but the commit failed. The
        // engine must reset, must not record an undoable expansion, and must
        // not emit the result again.
        let mut broker =
            BrokerFixture::start("emit-fails", "from-broker", Duration::from_millis(50));
        let config: Config = toml::from_str(BROKER_CONFIG).unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        adapter
            .engine_mut()
            .set_compatibility_mode(wayexpand_core::CompatibilityMode::TrackedFocus);
        let (completed, completion) = mpsc::channel();
        adapter.set_completion_notifier(Some(Arc::new(move || {
            let _ = completed.send(());
        })));
        adapter.focus_in();
        adapter.set_capabilities(crate::IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        for (key, before) in [(':', ""), ('o', ":"), ('k', ":o")] {
            let cursor = before.chars().count() as u32;
            adapter.set_surrounding_text(before, cursor, cursor);
            adapter.process_key_event(key as u32, 0, 0);
        }
        broker.answered();
        completion.recv_timeout(Duration::from_secs(5)).unwrap();

        let mut attempted = Vec::new();
        let result = adapter.drain_completed_commands_with(|actions| {
            attempted.extend_from_slice(actions);
            Err("commit signal failed")
        });
        assert_eq!(result, Err("commit signal failed"));
        assert_eq!(
            attempted,
            [
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("from-broker".into())
            ]
        );
        let undo = wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap();
        assert!(adapter.engine().prepare_undo(&undo).is_none());
        assert!(!adapter.process_key_event('x' as u32, 0, 0).handled);
        assert_eq!(
            adapter.drain_completed_commands_with(|_| Ok::<(), &str>(())),
            Ok(0)
        );
    }
}
