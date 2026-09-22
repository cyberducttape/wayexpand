use super::{IbusAction, IbusEngineAdapter};
use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::Duration,
};
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

/// IBus factory object. IBus creates one engine instance for this process; the
/// object path is stable and registered before the factory is advertised.
struct Factory;

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    fn create_engine(&self, name: &str) -> OwnedObjectPath {
        if name == "wayexpand" || name == "WayExpand" {
            OwnedObjectPath::try_from(ENGINE_PATH).expect("static object path")
        } else {
            OwnedObjectPath::try_from("/").expect("root object path")
        }
    }
}

struct EngineObject {
    adapter: Arc<Mutex<IbusEngineAdapter>>,
    connection: Arc<Mutex<Option<Connection>>>,
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
                    ENGINE_PATH,
                    "org.freedesktop.IBus.Engine",
                    "DeleteSurroundingText",
                    &(-(*nchars as i32), *nchars),
                ),
                IbusAction::CommitText(text) => {
                    let ibus_text = ibus_text_value(text);
                    connection.emit_signal(
                        None::<&str>,
                        ENGINE_PATH,
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
/// registers a factory and one engine object, and then remains alive while
/// zbus dispatches method calls on its internal async-io executor.
pub fn run_service(config_path: Option<std::path::PathBuf>) -> Result<(), IbusServiceError> {
    let path = config_path.unwrap_or_else(default_config_path);
    let store = ConfigStore::load(&path)?;
    let adapter = IbusEngineAdapter::new(ExpansionEngine::new((*store.config()).clone())?);
    let connection_slot = Arc::new(Mutex::new(None));
    let engine = EngineObject {
        adapter: Arc::new(Mutex::new(adapter)),
        connection: Arc::clone(&connection_slot),
    };
    let reload_receiver = store.subscribe();
    let reload_store = Arc::clone(&store);
    let reload_engine = Arc::clone(&engine.adapter);
    std::thread::Builder::new()
        .name("wayexpand-ibus-config".into())
        .spawn(move || loop {
            let _ = reload_store.reload_if_changed();
            while reload_receiver.try_recv().is_ok() {
                if let Ok(mut adapter) = reload_engine.lock() {
                    let _ = adapter.replace_config((*reload_store.config()).clone());
                }
            }
            std::thread::sleep(Duration::from_millis(250));
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
        .serve_at(FACTORY_PATH, Factory)?
        .serve_at(ENGINE_PATH, engine)?
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
    fn factory_accepts_the_advertised_engine_name() {
        let factory = Factory;
        assert_eq!(factory.create_engine("wayexpand").as_str(), ENGINE_PATH);
        assert_eq!(factory.create_engine("WayExpand").as_str(), ENGINE_PATH);
        assert_eq!(factory.create_engine("other").as_str(), "/");
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
        };
        engine.set_content_type(8, 0);
        assert!(!engine.process_key_event('a' as u32, 0, 0).unwrap());
        engine.set_content_type(0, 0);
        assert!(!engine.adapter.lock().unwrap().engine().is_sensitive_focus());
    }
}
