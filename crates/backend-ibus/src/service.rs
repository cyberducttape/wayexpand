use super::{IbusAction, IbusEngineAdapter};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use wayexpand_core::{default_config_path, Config, ExpansionEngine};
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
    adapter: Mutex<IbusEngineAdapter>,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl EngineObject {
    fn emit_actions(&self, actions: &[IbusAction]) {
        let Ok(connection) = self.connection.lock() else {
            return;
        };
        let Some(connection) = connection.as_ref() else {
            return;
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
            let _ = result;
        }
    }
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl EngineObject {
    fn process_key_event(&self, keyval: u32, keycode: u32, state: u32) -> bool {
        let Ok(mut adapter) = self.adapter.lock() else {
            return false;
        };
        let result = adapter.process_key_event(keyval, keycode, state);
        self.emit_actions(&result.actions);
        result.handled
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
    fn set_surrounding_text(&self, _text: OwnedValue, _cursor_pos: u32, _anchor_pos: u32) {}
}

/// Build and run the IBus engine process. The process owns a private bus name,
/// registers a factory and one engine object, and then remains alive while
/// zbus dispatches method calls on its internal async-io executor.
pub fn run_service(config_path: Option<std::path::PathBuf>) -> Result<(), IbusServiceError> {
    let path = config_path.unwrap_or_else(default_config_path);
    let config = Config::load(&path)?;
    let adapter = IbusEngineAdapter::new(ExpansionEngine::new(config)?);
    let connection_slot = Arc::new(Mutex::new(None));
    let engine = EngineObject {
        adapter: Mutex::new(adapter),
        connection: Arc::clone(&connection_slot),
    };
    let connection = Builder::session()?
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
