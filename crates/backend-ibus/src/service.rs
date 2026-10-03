use super::{IbusAction, IbusEngineAdapter};
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
use wayexpand_core::{default_config_path, ConfigStore, ExpansionEngine, OrganizationPolicy};
use zbus::{
    blocking::{connection::Builder, Connection},
    interface,
    zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value},
};

const BUS_NAME: &str = "org.freedesktop.IBus.Engine.wayexpand";
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

/// IBus factory state. Each CreateEngine call gets its own adapter and object
/// path; IBus may create multiple engines for separate input contexts. Engine
/// objects are removed from this map by their Destroy method.
struct Factory {
    connection: Arc<Mutex<Option<Connection>>>,
    config: Arc<Mutex<wayexpand_core::Config>>,
    policy: Arc<OrganizationPolicy>,
    instances: Arc<Mutex<HashMap<OwnedObjectPath, EngineInstance>>>,
    next_id: AtomicU64,
    /// Wakes the completion pump with the path of an engine whose
    /// asynchronous command finished.
    completions: mpsc::Sender<OwnedObjectPath>,
}

type Instances = Arc<Mutex<HashMap<OwnedObjectPath, EngineInstance>>>;

struct EngineInstance {
    adapter: Arc<Mutex<IbusEngineAdapter>>,
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
        let config = self
            .config
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus config lock poisoned".into()))?
            .clone();
        let engine = ExpansionEngine::new(config).map_err(|error| {
            zbus::fdo::Error::Failed(format!("could not create IBus engine: {error}"))
        })?;
        let mut adapter =
            IbusEngineAdapter::with_policy(engine, (*self.policy).clone()).map_err(|error| {
                zbus::fdo::Error::Failed(format!(
                    "administrator policy rejects IBus config: {error}"
                ))
            })?;
        let completions = self.completions.clone();
        let completed_path = path.clone();
        adapter.set_completion_notifier(Some(Arc::new(move || {
            let _ = completions.send(completed_path.clone());
        })));
        let adapter = Arc::new(Mutex::new(adapter));
        let engine = EngineObject {
            adapter: Arc::clone(&adapter),
            connection: Arc::clone(&self.connection),
            path: path.clone(),
            instances: Arc::clone(&self.instances),
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus connection lock poisoned".into()))?
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("IBus connection unavailable".into()))?;
        connection
            .inner()
            .object_server()
            .at(path.as_str(), engine)
            .await
            .map_err(zbus::fdo::Error::ZBus)?;
        self.instances
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?
            .insert(path.clone(), EngineInstance { adapter });
        Ok(path)
    }
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
                            &(Value::from(ibus_text_value(text)),),
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
                &(Value::from(ibus_text_value(text)),),
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
/// after its signals were emitted.
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
            Ok(mut adapter) => adapter.process_key_event(keyval, keycode, state),
            Err(_) => return false,
        };
        if self.emit_actions(&result.actions).await.is_err() {
            // The engine has already consumed this event. Reset its matcher
            // before returning false so a client retry cannot combine with a
            // half-applied replacement or stale trigger buffer.
            if let Ok(mut adapter) = self.adapter.lock() {
                adapter.reset();
            }
            return false;
        }
        result.handled
    }

    async fn focus_in(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_in();
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
            adapter.focus_in();
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
        let removed = self
            .instances
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?
            .remove(&self.path)
            .is_some();
        if !removed {
            return Ok(());
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
    fn set_content_type(&self, purpose: u32, hints: u32) {
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
    let initial_status = store.status();
    info!(
        config_state = initial_status.state,
        config_generation = initial_status.generation,
        "IBus configuration loaded"
    );
    let connection_slot = Arc::new(Mutex::new(None));
    let factory_config = Arc::new(Mutex::new((*store.config()).clone()));
    let instances: Instances = Arc::new(Mutex::new(HashMap::new()));
    let (completion_sender, completion_receiver) = mpsc::channel();
    let factory = Factory {
        connection: Arc::clone(&connection_slot),
        config: Arc::clone(&factory_config),
        policy,
        instances: Arc::clone(&instances),
        next_id: AtomicU64::new(1),
        completions: completion_sender,
    };
    let pump_instances = Arc::clone(&instances);
    let pump_connection = Arc::clone(&connection_slot);
    std::thread::Builder::new()
        .name("wayexpand-ibus-completions".into())
        .spawn(move || run_completion_pump(completion_receiver, pump_instances, pump_connection))
        .map_err(|error| IbusServiceError::Thread(error.to_string()))?;
    let reload_receiver = store.subscribe();
    let reload_store = Arc::clone(&store);
    let reload_config = Arc::clone(&factory_config);
    let reload_instances = Arc::clone(&instances);
    let config_watch_path = path.clone();
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
            let mut adapter_error = None;
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
                    let config = (*reload_store.config()).clone();
                    if let Ok(mut current) = reload_config.lock() {
                        *current = config.clone();
                    }
                    if let Ok(instances) = reload_instances.lock() {
                        for instance in instances.values() {
                            if let Ok(mut adapter) = instance.adapter.lock() {
                                if let Err(error) = adapter.replace_config(config.clone()) {
                                    let summary = error.safe_summary();
                                    if adapter_error.as_deref() != Some(summary.as_str()) {
                                        let status = reload_store.status();
                                        warn!(
                                            config_state = status.state,
                                            adapter_error = %summary,
                                            config_generation = status.generation,
                                            "IBus configuration could not be applied to an engine"
                                        );
                                        adapter_error = Some(summary);
                                    }
                                } else {
                                    adapter_error = None;
                                }
                            }
                        }
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
    *connection_slot.lock().expect("connection slot") = Some(connection.clone());
    loop {
        std::thread::park();
    }
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
    use wayexpand_core::Config;
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
        let factory = Factory {
            connection: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(config)),
            policy: Arc::new(OrganizationPolicy::default()),
            instances: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            completions: mpsc::channel().0,
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
        };
        engine.set_content_type(8, 0);
        assert!(!zbus::block_on(engine.process_key_event('a' as u32, 0, 0)));
        engine.set_content_type(0, 0);
        assert!(!engine.adapter.lock().unwrap().engine().is_sensitive_focus());
    }

    /// Broker-backed tests share the process-wide broker socket variable, so
    /// they must not overlap even when tests run on many threads.
    static BROKER_LOCK: Mutex<()> = Mutex::new(());

    const BROKER_CONFIG: &str = "[settings]\nundo_chord = \"Ctrl+Z\"\n\
         [[expansion]]\ntrigger = \":ok\"\nreplacement = \"\"\n\
         [expansion.command]\naction = \"status\"\ntimeout_ms = 3000\n";

    /// Serve exactly one broker request on `socket`, answering with `stdout`
    /// after `delay`. Returns the action id the engine asked for.
    fn fake_broker(
        socket: std::path::PathBuf,
        stdout: &'static str,
        delay: Duration,
    ) -> std::thread::JoinHandle<String> {
        use std::io::{BufRead, Write};
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            std::thread::sleep(delay);
            let response = serde_json::json!({ "Success": {
                "exit_code": 0, "stdout": stdout, "stderr": "",
                "stdout_truncated": false, "stderr_truncated": false, "duration_ms": 1,
            }});
            let mut stream = stream;
            let _ = writeln!(stream, "{response}");
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            request["action_id"].as_str().unwrap_or_default().to_owned()
        })
    }

    /// A broker answering one request, with the socket variable pointing at
    /// it for the lifetime of the value.
    struct BrokerFixture {
        _guard: std::sync::MutexGuard<'static, ()>,
        root: std::path::PathBuf,
        broker: Option<std::thread::JoinHandle<String>>,
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
            std::env::set_var("WAYEXPAND_ACTION_BROKER_SOCKET", &socket);
            let broker = Some(fake_broker(socket, stdout, delay));
            Self {
                _guard: guard,
                root,
                broker,
            }
        }

        /// Wait until the broker has answered; returns the requested action.
        fn answered(&mut self) -> String {
            self.broker.take().unwrap().join().unwrap()
        }
    }

    impl Drop for BrokerFixture {
        fn drop(&mut self) {
            std::env::remove_var("WAYEXPAND_ACTION_BROKER_SOCKET");
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
        instances: Instances,
    }

    impl Harness {
        fn start(config: &str) -> Self {
            let config: Config = toml::from_str(config).unwrap();
            let connection_slot = Arc::new(Mutex::new(None));
            let instances: Instances = Arc::new(Mutex::new(HashMap::new()));
            let (completion_sender, completion_receiver) = mpsc::channel();
            let factory = Factory {
                connection: Arc::clone(&connection_slot),
                config: Arc::new(Mutex::new(config)),
                policy: Arc::new(OrganizationPolicy::default()),
                instances: Arc::clone(&instances),
                next_id: AtomicU64::new(1),
                completions: completion_sender,
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
            {
                let instances = Arc::clone(&instances);
                let connection = Arc::clone(&connection_slot);
                std::thread::spawn(move || {
                    run_completion_pump(completion_receiver, instances, connection)
                });
            }
            let (commit_sender, commits) = mpsc::channel();
            let messages = zbus::blocking::MessageIterator::from(&client);
            std::thread::spawn(move || {
                for message in messages.flatten() {
                    let header = message.header();
                    if header.member().map(|member| member.as_str()) == Some("CommitText") {
                        let (text,): (OwnedValue,) = message.body().deserialize().unwrap();
                        let _ = commit_sender.send(ibus_text_string(&text).unwrap());
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
                instances,
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

        /// The sequence ibus-daemon sends when an ordinary text field gains
        /// focus.
        fn focus_text_field(&self) {
            self.call("FocusIn", &());
            self.call("Enable", &());
            self.call("SetCapabilities", &(crate::IBUS_CAP_SURROUNDING_TEXT,));
            self.call("SetContentType", &(0_u32, 0_u32));
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
    }

    #[test]
    fn broker_result_reaches_the_ibus_client_through_the_completion_pump() {
        let mut broker =
            BrokerFixture::start("delivered", "from-broker", Duration::from_millis(150));
        let harness = Harness::start(BROKER_CONFIG);
        harness.focus_text_field();
        harness.type_trigger();
        assert_eq!(broker.answered(), "status");
        assert_eq!(
            harness.commits(Duration::from_secs(2)),
            ["o", "k", "from-broker"]
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
        harness.call("SetContentType", &(8_u32, 0_u32));
        broker.answered();
        assert_eq!(harness.commits(Duration::from_millis(500)), ["o", "k"]);
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
        assert_eq!(
            adapter.drain_completed_commands_with(|_| Ok::<(), &str>(())),
            Ok(0)
        );
    }
}
