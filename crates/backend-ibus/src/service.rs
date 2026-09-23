use super::{IbusAction, IbusEngineAdapter};
use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tracing::{info, warn};
use wayexpand_core::{default_config_path, ConfigStore, ExpansionEngine};
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
}

/// IBus factory state. Each CreateEngine call gets its own adapter and object
/// path; IBus may create multiple engines for separate input contexts.
struct Factory {
    connection: Arc<Mutex<Option<Connection>>>,
    config: Arc<Mutex<wayexpand_core::Config>>,
    instances: Arc<Mutex<Vec<Weak<Mutex<IbusEngineAdapter>>>>>,
    next_id: AtomicU64,
}

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    fn create_engine(&self, name: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if name != "wayexpand" && name != "WayExpand" {
            return Ok(OwnedObjectPath::try_from("/").expect("root object path"));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let path = engine_path(id);
        let config = self
            .config
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus config lock poisoned".into()))?
            .clone();
        let adapter = Arc::new(Mutex::new(IbusEngineAdapter::new(
            ExpansionEngine::new(config).map_err(|error| {
                zbus::fdo::Error::Failed(format!("could not create IBus engine: {error}"))
            })?,
        )));
        let engine = EngineObject {
            adapter: Arc::clone(&adapter),
            connection: Arc::clone(&self.connection),
            path: path.clone(),
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus connection lock poisoned".into()))?
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("IBus connection unavailable".into()))?;
        connection
            .object_server()
            .at(path.as_str(), engine)
            .map_err(zbus::fdo::Error::ZBus)?;
        self.instances
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("IBus instance lock poisoned".into()))?
            .push(Arc::downgrade(&adapter));
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
}

impl EngineObject {
    fn emit_actions(&self, actions: &[IbusAction]) -> zbus::fdo::Result<()> {
        let Ok(connection) = self.connection.lock() else {
            return Err(zbus::fdo::Error::Failed(
                "IBus connection lock poisoned".into(),
            ));
        };
        let Some(connection) = connection.as_ref() else {
            return Err(zbus::fdo::Error::Failed(
                "IBus connection unavailable".into(),
            ));
        };
        for action in actions {
            let result = match action {
                IbusAction::DeleteSurroundingText { nchars } => connection.emit_signal(
                    None::<&str>,
                    self.path.as_str(),
                    "org.freedesktop.IBus.Engine",
                    "DeleteSurroundingText",
                    &(-(*nchars as i32), *nchars),
                ),
                IbusAction::CommitText(text) => {
                    let ibus_text = ibus_text_value(text);
                    connection.emit_signal(
                        None::<&str>,
                        self.path.as_str(),
                        "org.freedesktop.IBus.Engine",
                        "CommitText",
                        &(Value::from(ibus_text),),
                    )
                }
            };
            result.map_err(zbus::fdo::Error::ZBus)?;
        }
        Ok(())
    }
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl EngineObject {
    fn process_key_event(&self, keyval: u32, keycode: u32, state: u32) -> zbus::fdo::Result<bool> {
        let Ok(mut adapter) = self.adapter.lock() else {
            return Ok(false);
        };
        let result = adapter.process_key_event(keyval, keycode, state);
        if let Err(_error) = self.emit_actions(&result.actions) {
            // The engine has already consumed this event. Reset its matcher
            // before returning false so a client retry cannot combine with a
            // half-applied replacement or stale trigger buffer.
            adapter.reset();
            return Ok(false);
        }
        Ok(result.handled)
    }

    fn focus_in(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_in();
        }
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
    fn enable(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_in();
        }
    }
    fn disable(&self) {
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter.focus_out();
        }
    }
    fn set_cursor_location(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
    fn set_capabilities(&self, _caps: u32) {}
    /// IBus purpose values 8 and 9 are PASSWORD and PIN respectively. Both
    /// hide user input and must disable expansion before the next key arrives.
    fn set_content_type(&self, purpose: u32, _hints: u32) {
        let sensitive = matches!(purpose, 8 | 9);
        if let Ok(mut adapter) = self.adapter.lock() {
            adapter
                .engine_mut()
                .process(wayexpand_core::InputEvent::FocusChanged { sensitive });
        }
    }
    fn set_surrounding_text(&self, _text: OwnedValue, _cursor_pos: u32, _anchor_pos: u32) {}
}

/// Build and run the IBus engine process. The process owns a private bus name,
/// registers a factory, and creates one isolated engine object per request
/// while zbus dispatches method calls on its internal async-io executor.
pub fn run_service(config_path: Option<std::path::PathBuf>) -> Result<(), IbusServiceError> {
    let path = config_path.unwrap_or_else(default_config_path);
    let store = ConfigStore::load(&path)?;
    let initial_status = store.status();
    info!(
        config_state = initial_status.state,
        config_generation = initial_status.generation,
        "IBus configuration loaded"
    );
    let connection_slot = Arc::new(Mutex::new(None));
    let factory_config = Arc::new(Mutex::new((*store.config()).clone()));
    let instances: Arc<Mutex<Vec<Weak<Mutex<IbusEngineAdapter>>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let factory = Factory {
        connection: Arc::clone(&connection_slot),
        config: Arc::clone(&factory_config),
        instances: Arc::clone(&instances),
        next_id: AtomicU64::new(1),
    };
    let reload_receiver = store.subscribe();
    let reload_store = Arc::clone(&store);
    let reload_config = Arc::clone(&factory_config);
    std::thread::Builder::new()
        .name("wayexpand-ibus-config".into())
        .spawn(move || {
            let mut reload_error = None;
            let mut adapter_error = None;
            loop {
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
                    if let Ok(mut instances) = instances.lock() {
                        instances.retain(|weak| {
                            let Some(adapter) = weak.upgrade() else {
                                return false;
                            };
                            if let Ok(mut adapter) = adapter.lock() {
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
                            true
                        });
                    }
                }
                std::thread::sleep(Duration::from_millis(250));
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
        };
        engine.set_content_type(8, 0);
        assert!(!engine.process_key_event('a' as u32, 0, 0).unwrap());
        engine.set_content_type(0, 0);
        assert!(!engine.adapter.lock().unwrap().engine().is_sensitive_focus());
    }
}
