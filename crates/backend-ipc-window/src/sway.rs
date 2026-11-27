//! Sway focused-window tracking over the i3-compatible IPC socket.
//!
//! One connection subscribes to `window` and `workspace` events; requests
//! (`GET_TREE`, `GET_VERSION`) use short-lived connections of their own so a
//! reply can never interleave with an event.

use std::env;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::Value;
use wayexpand_core::{WindowContext, WindowIdentitySource, WindowTracker, WindowTrackerError};

use crate::{check_cancelled, fill_until, lost, window_context, Filled, IpcWindowError};

const BACKEND_NAME: &str = "sway-ipc";
const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER_LEN: usize = MAGIC.len() + 8;
/// The full tree of a busy session is the largest message read here.
const MESSAGE_LIMIT: usize = 16 * 1024 * 1024;

const SUBSCRIBE: u32 = 2;
const GET_TREE: u32 = 4;
const GET_VERSION: u32 = 7;
const EVENT_BIT: u32 = 0x8000_0000;
const EVENT_WORKSPACE: u32 = EVENT_BIT;
const EVENT_WINDOW: u32 = EVENT_BIT | 3;
const EVENT_SHUTDOWN: u32 = EVENT_BIT | 0x10;

/// The socket path Sway advertises to its clients.
fn socket_path() -> Result<PathBuf, IpcWindowError> {
    env::var_os("SWAYSOCK")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(IpcWindowError::NotInSession("SWAYSOCK", "Sway"))
}

fn encode(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER_LEN + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    message
}

/// Take one complete message off the front of `buffer`, if it holds one.
fn take_message(buffer: &mut Vec<u8>) -> Result<Option<(u32, Vec<u8>)>, IpcWindowError> {
    if buffer.len() < HEADER_LEN {
        return Ok(None);
    }
    if &buffer[..MAGIC.len()] != MAGIC {
        return Err(IpcWindowError::Protocol("missing i3-ipc magic".into()));
    }
    let word = |offset: usize| {
        let mut bytes = [0_u8; 4];
        bytes.copy_from_slice(&buffer[offset..offset + 4]);
        u32::from_ne_bytes(bytes)
    };
    let length = word(MAGIC.len()) as usize;
    let kind = word(MAGIC.len() + 4);
    if length > MESSAGE_LIMIT {
        return Err(IpcWindowError::Protocol("oversized message".into()));
    }
    if buffer.len() < HEADER_LEN + length {
        return Ok(None);
    }
    let payload = buffer[HEADER_LEN..HEADER_LEN + length].to_vec();
    buffer.drain(..HEADER_LEN + length);
    Ok(Some((kind, payload)))
}

/// Send one request on `stream` and wait for its reply.
fn exchange(stream: &mut UnixStream, kind: u32, payload: &[u8]) -> Result<Value, IpcWindowError> {
    stream.set_write_timeout(Some(crate::REQUEST_TIMEOUT))?;
    stream.write_all(&encode(kind, payload))?;
    let deadline = Instant::now() + crate::REQUEST_TIMEOUT;
    let mut buffer = Vec::new();
    loop {
        if let Some((reply_kind, body)) = take_message(&mut buffer)? {
            if reply_kind != kind {
                return Err(IpcWindowError::Protocol(format!(
                    "expected reply type {kind}, got {reply_kind}"
                )));
            }
            return serde_json::from_slice(&body)
                .map_err(|error| IpcWindowError::Protocol(error.to_string()));
        }
        if let Filled::TimedOut = fill_until(stream, &mut buffer, deadline, MESSAGE_LIMIT)? {
            return Err(IpcWindowError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Sway did not answer in time",
            )));
        }
    }
}

fn request(socket: &Path, kind: u32) -> Result<Value, IpcWindowError> {
    let mut stream = UnixStream::connect(socket)?;
    exchange(&mut stream, kind, b"")
}

/// A focused application window, reduced to what the engine may use.
#[derive(Debug, Clone, PartialEq)]
struct Focused {
    id: Option<i64>,
    context: Option<WindowContext>,
    identity: WindowIdentitySource,
}

impl Focused {
    fn nothing() -> Self {
        Self {
            id: None,
            context: None,
            identity: WindowIdentitySource::Unavailable,
        }
    }
}

/// Describe a Sway container. Native Wayland windows carry an xdg `app_id`;
/// Xwayland windows carry an X11 class in `window_properties` instead.
fn describe(container: &Value, include_titles: bool) -> Focused {
    let kind = container.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("con" | "floating_con")) {
        return Focused::nothing();
    }
    let id = container.get("id").and_then(Value::as_i64);
    let native = container
        .get("app_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let class = container
        .get("window_properties")
        .and_then(|properties| properties.get("class"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let (app_id, identity) = match (native, class) {
        (Some(app_id), _) => (Some(app_id), WindowIdentitySource::DesktopFileName),
        (None, Some(class)) => (Some(class), WindowIdentitySource::ResourceClass),
        (None, None) => (None, WindowIdentitySource::Unavailable),
    };
    let title = include_titles
        .then(|| container.get("name").and_then(Value::as_str))
        .flatten();
    let context = window_context(app_id, title, id.map(|id| id.to_string()));
    Focused {
        id,
        identity: if context.is_some() {
            identity
        } else {
            WindowIdentitySource::Unavailable
        },
        context,
    }
}

/// Find the focused node anywhere in a `GET_TREE` reply.
fn focused_node(node: &Value) -> Option<&Value> {
    if node.get("focused").and_then(Value::as_bool) == Some(true) {
        return Some(node);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .find_map(focused_node)
}

fn focused_from_tree(tree: &Value, include_titles: bool) -> Focused {
    focused_node(tree)
        .map(|node| describe(node, include_titles))
        .unwrap_or_else(Focused::nothing)
}

pub struct SwayWindowTracker {
    socket: PathBuf,
    events: UnixStream,
    buffer: Vec<u8>,
    include_titles: bool,
    current: Focused,
    pending: Option<Option<WindowContext>>,
}

impl SwayWindowTracker {
    /// A bounded, side-effect-free check that Sway answers on `$SWAYSOCK`.
    pub fn probe() -> Result<(), IpcWindowError> {
        let version = request(&socket_path()?, GET_VERSION)?;
        if version.get("major").and_then(Value::as_i64).is_none() {
            return Err(IpcWindowError::Protocol("GET_VERSION has no major".into()));
        }
        Ok(())
    }

    pub fn new() -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(None, false)
    }

    pub fn new_cancellable(cancelled: Option<&AtomicBool>) -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(cancelled, false)
    }

    /// Titles are read only when the configuration has a `title_contains`
    /// filter; app-ID matching needs none.
    pub fn new_with_titles(include_titles: bool) -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(None, include_titles)
    }

    pub fn new_cancellable_with_titles(
        cancelled: Option<&AtomicBool>,
        include_titles: bool,
    ) -> Result<Self, IpcWindowError> {
        Self::connect(&socket_path()?, cancelled, include_titles)
    }

    fn connect(
        socket: &Path,
        cancelled: Option<&AtomicBool>,
        include_titles: bool,
    ) -> Result<Self, IpcWindowError> {
        check_cancelled(cancelled)?;
        let mut events = UnixStream::connect(socket)?;
        // Subscribe before reading the tree, so a focus change between the
        // two is delivered as an event instead of being lost.
        let reply = exchange(
            &mut events,
            SUBSCRIBE,
            br#"["window","workspace","shutdown"]"#,
        )?;
        if reply.get("success").and_then(Value::as_bool) != Some(true) {
            return Err(IpcWindowError::SubscriptionRefused);
        }
        check_cancelled(cancelled)?;
        let current = focused_from_tree(&request(socket, GET_TREE)?, include_titles);
        let pending = Some(current.context.clone());
        Ok(Self {
            socket: socket.to_owned(),
            events,
            buffer: Vec::new(),
            include_titles,
            current,
            pending,
        })
    }

    /// Record `next` and return it if it differs from what was last sent.
    fn update(&mut self, next: Focused) -> Option<Option<WindowContext>> {
        if next == self.current {
            return None;
        }
        self.current = next;
        Some(self.current.context.clone())
    }

    fn handle(
        &mut self,
        kind: u32,
        payload: &[u8],
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        match kind {
            EVENT_SHUTDOWN => Err(lost(BACKEND_NAME, "Sway is shutting down")),
            EVENT_WINDOW => {
                let event: Value = serde_json::from_slice(payload)
                    .map_err(|error| lost(BACKEND_NAME, format!("bad window event: {error}")))?;
                let change = event.get("change").and_then(Value::as_str);
                let Some(container) = event.get("container") else {
                    return Ok(None);
                };
                let event_id = container.get("id").and_then(Value::as_i64);
                let is_current = event_id.is_some() && event_id == self.current.id;
                match change {
                    Some("focus") => Ok(self.update(describe(container, self.include_titles))),
                    Some("title") if is_current && self.include_titles => {
                        Ok(self.update(describe(container, true)))
                    }
                    // A newly focused window follows with its own event; until
                    // then nothing is focused, so filters fail closed.
                    Some("close") if is_current => Ok(self.update(Focused::nothing())),
                    _ => Ok(None),
                }
            }
            EVENT_WORKSPACE => {
                // Switching to an empty workspace sends no window event, so
                // re-read what holds focus now.
                let tree = request(&self.socket, GET_TREE)
                    .map_err(|error| lost(BACKEND_NAME, error.to_string()))?;
                Ok(self.update(focused_from_tree(&tree, self.include_titles)))
            }
            _ => Ok(None),
        }
    }
}

impl WindowTracker for SwayWindowTracker {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn identity_source(&self) -> WindowIdentitySource {
        self.current.identity
    }

    fn next_window_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        if let Some(initial) = self.pending.take() {
            return Ok(Some(initial));
        }
        let deadline = Instant::now() + timeout;
        loop {
            while let Some((kind, payload)) = take_message(&mut self.buffer)
                .map_err(|error| lost(BACKEND_NAME, error.to_string()))?
            {
                if let Some(update) = self.handle(kind, &payload)? {
                    return Ok(Some(update));
                }
            }
            match fill_until(&mut self.events, &mut self.buffer, deadline, MESSAGE_LIMIT) {
                Ok(Filled::Data) => {}
                Ok(Filled::TimedOut) => return Ok(None),
                Err(error) => return Err(lost(BACKEND_NAME, error.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use serde_json::json;
    use std::io::Read;
    use std::os::unix::net::UnixListener;
    use std::thread;

    fn read_request(stream: &mut UnixStream) -> (u32, Vec<u8>) {
        let mut header = [0_u8; HEADER_LEN];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(&header[..6], MAGIC);
        let length = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
        let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).unwrap();
        (kind, payload)
    }

    fn send(stream: &mut UnixStream, kind: u32, value: &Value) {
        stream
            .write_all(&encode(kind, value.to_string().as_bytes()))
            .unwrap();
    }

    fn window(id: i64, app_id: Option<&str>, class: Option<&str>, name: &str) -> Value {
        json!({
            "id": id, "type": "con", "focused": true, "name": name,
            "app_id": app_id,
            "window_properties": class.map(|class| json!({"class": class})),
        })
    }

    fn tree(focused: Value) -> Value {
        json!({
            "id": 1, "type": "root", "focused": false,
            "nodes": [{"id": 2, "type": "output", "focused": false, "nodes": [
                {"id": 3, "type": "workspace", "focused": false,
                 "nodes": [], "floating_nodes": [focused]}
            ]}]
        })
    }

    /// A scripted Sway: answers the subscription and first tree request, then
    /// hands the event connection and later tree requests to `script`.
    fn fake_sway(
        initial: Value,
        script: impl FnOnce(UnixStream, &UnixListener) + Send + 'static,
    ) -> (TempDir, thread::JoinHandle<()>) {
        let dir = TempDir::new();
        let listener = UnixListener::bind(dir.path().join("sway.sock")).unwrap();
        let handle = thread::spawn(move || {
            let (mut events, _) = listener.accept().unwrap();
            let (kind, payload) = read_request(&mut events);
            assert_eq!(kind, SUBSCRIBE);
            let topics: Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(topics, json!(["window", "workspace", "shutdown"]));
            send(&mut events, SUBSCRIBE, &json!({"success": true}));
            let (mut query, _) = listener.accept().unwrap();
            assert_eq!(read_request(&mut query).0, GET_TREE);
            send(&mut query, GET_TREE, &initial);
            script(events, &listener);
        });
        (dir, handle)
    }

    fn connect(dir: &TempDir, titles: bool) -> SwayWindowTracker {
        SwayWindowTracker::connect(&dir.path().join("sway.sock"), None, titles).unwrap()
    }

    fn next(tracker: &mut SwayWindowTracker) -> Option<Option<WindowContext>> {
        tracker
            .next_window_timeout(Duration::from_secs(2))
            .expect("tracker stays connected")
    }

    #[test]
    fn reports_the_initial_window_then_focus_changes_without_titles() {
        let (dir, server) = fake_sway(
            tree(window(10, Some("org.kde.kate"), None, "secret.txt")),
            |mut events, _| {
                let firefox = window(11, None, Some("firefox"), "Bank login");
                // Split one event across two writes to exercise buffering.
                let bytes = encode(
                    EVENT_WINDOW,
                    json!({"change": "focus", "container": firefox})
                        .to_string()
                        .as_bytes(),
                );
                events.write_all(&bytes[..9]).unwrap();
                thread::sleep(Duration::from_millis(50));
                events.write_all(&bytes[9..]).unwrap();
                thread::sleep(Duration::from_millis(200));
            },
        );
        let mut tracker = connect(&dir, false);
        let initial = next(&mut tracker).unwrap().unwrap();
        assert_eq!(initial.app_id.as_deref(), Some("org.kde.kate"));
        assert_eq!(initial.title, None, "titles are not read unless asked for");
        assert_eq!(initial.instance_id.as_deref(), Some("10"));
        assert_eq!(
            tracker.identity_source(),
            WindowIdentitySource::DesktopFileName
        );
        let firefox = next(&mut tracker).unwrap().unwrap();
        assert_eq!(firefox.app_id.as_deref(), Some("firefox"));
        assert_eq!(
            tracker.identity_source(),
            WindowIdentitySource::ResourceClass
        );
        server.join().unwrap();
    }

    #[test]
    fn an_empty_workspace_clears_the_focused_window() {
        let (dir, server) = fake_sway(
            tree(window(10, Some("foot"), None, "")),
            |mut events, listener| {
                send(&mut events, EVENT_WORKSPACE, &json!({"change": "focus"}));
                let (mut query, _) = listener.accept().unwrap();
                assert_eq!(read_request(&mut query).0, GET_TREE);
                send(
                    &mut query,
                    GET_TREE,
                    &json!({"id": 1, "type": "root", "focused": false, "nodes": [
                        {"id": 4, "type": "workspace", "focused": true, "nodes": []}
                    ]}),
                );
                thread::sleep(Duration::from_millis(200));
            },
        );
        let mut tracker = connect(&dir, false);
        assert!(next(&mut tracker).unwrap().is_some());
        assert_eq!(next(&mut tracker), Some(None));
        assert_eq!(tracker.identity_source(), WindowIdentitySource::Unavailable);
        server.join().unwrap();
    }

    #[test]
    fn title_changes_are_reported_only_when_titles_are_requested() {
        for titles in [false, true] {
            let (dir, server) = fake_sway(
                tree(window(10, Some("foot"), None, "one")),
                |mut events, _| {
                    send(
                        &mut events,
                        EVENT_WINDOW,
                        &json!({"change": "title", "container": window(10, Some("foot"), None, "two")}),
                    );
                    thread::sleep(Duration::from_millis(300));
                },
            );
            let mut tracker = connect(&dir, titles);
            next(&mut tracker);
            let update = tracker
                .next_window_timeout(Duration::from_millis(200))
                .unwrap();
            if titles {
                assert_eq!(update.unwrap().unwrap().title.as_deref(), Some("two"));
            } else {
                assert_eq!(update, None);
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn closing_the_focused_window_fails_closed_and_shutdown_disconnects() {
        let (dir, server) = fake_sway(tree(window(10, Some("foot"), None, "")), |mut events, _| {
            // Closing some other window changes nothing.
            send(
                &mut events,
                EVENT_WINDOW,
                &json!({"change": "close", "container": window(99, Some("x"), None, "")}),
            );
            send(
                &mut events,
                EVENT_WINDOW,
                &json!({"change": "close", "container": window(10, Some("foot"), None, "")}),
            );
            send(&mut events, EVENT_SHUTDOWN, &json!({"change": "exit"}));
            thread::sleep(Duration::from_millis(200));
        });
        let mut tracker = connect(&dir, false);
        next(&mut tracker);
        assert_eq!(next(&mut tracker), Some(None));
        let error = tracker
            .next_window_timeout(Duration::from_secs(2))
            .unwrap_err();
        assert!(error.retryable);
        server.join().unwrap();
    }

    #[test]
    fn a_closed_socket_is_a_retryable_disconnect() {
        let (dir, server) = fake_sway(tree(window(10, Some("foot"), None, "")), |events, _| {
            drop(events)
        });
        let mut tracker = connect(&dir, false);
        next(&mut tracker);
        server.join().unwrap();
        assert!(
            tracker
                .next_window_timeout(Duration::from_secs(2))
                .unwrap_err()
                .retryable
        );
    }

    #[test]
    fn errors_never_contain_window_identity() {
        let event = json!({"change": "focus", "container": window(1, Some("secret-app"), None, "secret title")});
        let mut buffer = encode(EVENT_WINDOW, event.to_string().as_bytes());
        buffer[0] = b'x';
        let error = take_message(&mut buffer).unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
}
