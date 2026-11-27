//! Hyprland focused-window tracking.
//!
//! Hyprland publishes events as `NAME>>DATA` lines on `.socket2.sock` and
//! answers one request per connection on `.socket.sock`. Focus events carry
//! only a window address, so each change is resolved with `j/activewindow`.

use std::env;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::Value;
use wayexpand_core::{WindowContext, WindowIdentitySource, WindowTracker, WindowTrackerError};

use crate::{check_cancelled, fill_until, lost, window_context, Filled, IpcWindowError};

const BACKEND_NAME: &str = "hyprland-ipc";
const REPLY_LIMIT: usize = 1024 * 1024;
/// A single event line is short; anything longer is not Hyprland.
const LINE_LIMIT: usize = 64 * 1024;

/// Hyprland's per-instance socket directory. Current releases use
/// `$XDG_RUNTIME_DIR/hypr/<signature>`; releases before 0.40 used `/tmp/hypr`.
fn instance_dir() -> Result<PathBuf, IpcWindowError> {
    let signature = env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .filter(|value| !value.is_empty())
        .ok_or(IpcWindowError::NotInSession(
            "HYPRLAND_INSTANCE_SIGNATURE",
            "Hyprland",
        ))?;
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(|runtime| PathBuf::from(runtime).join("hypr").join(&signature));
    let legacy = Path::new("/tmp/hypr").join(&signature);
    Ok(runtime
        .filter(|dir| dir.join(".socket2.sock").exists())
        .unwrap_or_else(|| {
            if legacy.join(".socket2.sock").exists() {
                legacy
            } else {
                // Report the current location in errors.
                env::var_os("XDG_RUNTIME_DIR")
                    .map(|runtime| PathBuf::from(runtime).join("hypr").join(&signature))
                    .unwrap_or(legacy)
            }
        }))
}

fn request(dir: &Path, command: &str) -> Result<Value, IpcWindowError> {
    let mut stream = UnixStream::connect(dir.join(".socket.sock"))?;
    stream.set_write_timeout(Some(crate::REQUEST_TIMEOUT))?;
    stream.write_all(command.as_bytes())?;
    let deadline = Instant::now() + crate::REQUEST_TIMEOUT;
    let mut reply = Vec::new();
    // Hyprland closes the connection after its reply.
    loop {
        match fill_until(&mut stream, &mut reply, deadline, REPLY_LIMIT) {
            Ok(Filled::Data) => {}
            Ok(Filled::TimedOut) => {
                return Err(IpcWindowError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Hyprland did not answer in time",
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        }
    }
    serde_json::from_slice(&reply).map_err(|error| IpcWindowError::Protocol(error.to_string()))
}

fn normalize_address(address: &str) -> Option<String> {
    let address = address.trim().trim_start_matches("0x");
    (!address.is_empty() && address != ",").then(|| address.to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq)]
struct Focused {
    address: Option<String>,
    context: Option<WindowContext>,
    identity: WindowIdentitySource,
}

impl Focused {
    fn nothing() -> Self {
        Self {
            address: None,
            context: None,
            identity: WindowIdentitySource::Unavailable,
        }
    }
}

/// Describe a `j/activewindow` reply (`{}` when nothing is focused).
fn describe(window: &Value, include_titles: bool) -> Focused {
    let Some(address) = window
        .get("address")
        .and_then(Value::as_str)
        .and_then(normalize_address)
    else {
        return Focused::nothing();
    };
    let class = window
        .get("class")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            window
                .get("initialClass")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        });
    let xwayland = window.get("xwayland").and_then(Value::as_bool) == Some(true);
    let title = include_titles
        .then(|| window.get("title").and_then(Value::as_str))
        .flatten();
    let context = window_context(class, title, Some(address.clone()));
    let identity = match (&context, class, xwayland) {
        (Some(_), Some(_), true) => WindowIdentitySource::ResourceClass,
        (Some(_), Some(_), false) => WindowIdentitySource::DesktopFileName,
        _ => WindowIdentitySource::Unavailable,
    };
    Focused {
        address: Some(address),
        context,
        identity,
    }
}

pub struct HyprlandWindowTracker {
    dir: PathBuf,
    events: UnixStream,
    buffer: Vec<u8>,
    include_titles: bool,
    current: Focused,
    pending: Option<Option<WindowContext>>,
}

impl HyprlandWindowTracker {
    /// A bounded, side-effect-free check that Hyprland answers requests.
    pub fn probe() -> Result<(), IpcWindowError> {
        let version = request(&instance_dir()?, "j/version")?;
        if version.get("tag").is_none() && version.get("version").is_none() {
            return Err(IpcWindowError::Protocol("j/version has no version".into()));
        }
        Ok(())
    }

    pub fn new() -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(None, false)
    }

    pub fn new_cancellable(cancelled: Option<&AtomicBool>) -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(cancelled, false)
    }

    pub fn new_with_titles(include_titles: bool) -> Result<Self, IpcWindowError> {
        Self::new_cancellable_with_titles(None, include_titles)
    }

    pub fn new_cancellable_with_titles(
        cancelled: Option<&AtomicBool>,
        include_titles: bool,
    ) -> Result<Self, IpcWindowError> {
        Self::connect(&instance_dir()?, cancelled, include_titles)
    }

    fn connect(
        dir: &Path,
        cancelled: Option<&AtomicBool>,
        include_titles: bool,
    ) -> Result<Self, IpcWindowError> {
        check_cancelled(cancelled)?;
        // Listen before asking, so a change in between is not lost.
        let events = UnixStream::connect(dir.join(".socket2.sock"))?;
        check_cancelled(cancelled)?;
        let current = describe(&request(dir, "j/activewindow")?, include_titles);
        let pending = Some(current.context.clone());
        Ok(Self {
            dir: dir.to_owned(),
            events,
            buffer: Vec::new(),
            include_titles,
            current,
            pending,
        })
    }

    fn update(&mut self, next: Focused) -> Option<Option<WindowContext>> {
        if next == self.current {
            return None;
        }
        self.current = next;
        Some(self.current.context.clone())
    }

    fn refresh(&mut self) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        let window = request(&self.dir, "j/activewindow")
            .map_err(|error| lost(BACKEND_NAME, error.to_string()))?;
        let next = describe(&window, self.include_titles);
        Ok(self.update(next))
    }

    fn is_current(&self, address: &str) -> bool {
        normalize_address(address).is_some_and(|address| self.current.address == Some(address))
    }

    fn handle(&mut self, line: &str) -> Result<Option<Option<WindowContext>>, WindowTrackerError> {
        let Some((event, data)) = line.split_once(">>") else {
            return Ok(None);
        };
        match event {
            "activewindowv2" => match normalize_address(data) {
                None => Ok(self.update(Focused::nothing())),
                Some(_) => self.refresh(),
            },
            "windowtitlev2" | "windowtitle" if self.include_titles => {
                let address = data.split(',').next().unwrap_or_default();
                if self.is_current(address) {
                    self.refresh()
                } else {
                    Ok(None)
                }
            }
            // The next focused window follows with its own event.
            "closewindow" if self.is_current(data) => Ok(self.update(Focused::nothing())),
            // A window can change class after mapping (some toolkits set it
            // late); Hyprland reports that only through a fresh query.
            "windowclass" | "changefloatingmode" if self.is_current(data) => self.refresh(),
            _ => Ok(None),
        }
    }
}

impl WindowTracker for HyprlandWindowTracker {
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
            while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line[..end]).into_owned();
                if let Some(update) = self.handle(&line)? {
                    return Ok(Some(update));
                }
            }
            match fill_until(&mut self.events, &mut self.buffer, deadline, LINE_LIMIT) {
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
    use std::sync::mpsc;
    use std::thread;

    /// A scripted Hyprland: the request socket answers `j/activewindow`
    /// from a queue; the event socket is handed to the test.
    struct FakeHyprland {
        dir: TempDir,
        answers: mpsc::Sender<Value>,
        events: mpsc::Receiver<UnixStream>,
    }

    impl FakeHyprland {
        fn start() -> Self {
            let dir = TempDir::new();
            let requests = UnixListener::bind(dir.path().join(".socket.sock")).unwrap();
            let events = UnixListener::bind(dir.path().join(".socket2.sock")).unwrap();
            let (answers, queue) = mpsc::channel::<Value>();
            thread::spawn(move || {
                for answer in queue {
                    let Ok((mut stream, _)) = requests.accept() else {
                        return;
                    };
                    let mut command = [0_u8; 64];
                    let count = stream.read(&mut command).unwrap();
                    assert_eq!(&command[..count], b"j/activewindow");
                    stream.write_all(answer.to_string().as_bytes()).unwrap();
                }
            });
            let (event_sender, event_streams) = mpsc::channel();
            thread::spawn(move || {
                if let Ok((stream, _)) = events.accept() {
                    let _ = event_sender.send(stream);
                }
            });
            Self {
                dir,
                answers,
                events: event_streams,
            }
        }

        fn answer(&self, window: Value) {
            self.answers.send(window).unwrap();
        }

        fn connect(&self, titles: bool) -> (HyprlandWindowTracker, UnixStream) {
            let tracker = HyprlandWindowTracker::connect(self.dir.path(), None, titles).unwrap();
            let events = self.events.recv_timeout(Duration::from_secs(2)).unwrap();
            (tracker, events)
        }
    }

    fn window(address: &str, class: &str, title: &str, xwayland: bool) -> Value {
        json!({"address": address, "class": class, "title": title,
               "initialClass": class, "xwayland": xwayland, "pid": 42})
    }

    fn next(tracker: &mut HyprlandWindowTracker) -> Option<Option<WindowContext>> {
        tracker
            .next_window_timeout(Duration::from_secs(2))
            .expect("tracker stays connected")
    }

    #[test]
    fn reports_the_initial_window_and_each_focus_change() {
        let hypr = FakeHyprland::start();
        hypr.answer(window("0x55aa", "kitty", "secret", false));
        let (mut tracker, mut events) = hypr.connect(false);
        let initial = next(&mut tracker).unwrap().unwrap();
        assert_eq!(initial.app_id.as_deref(), Some("kitty"));
        assert_eq!(initial.title, None, "titles are not read unless asked for");
        assert_eq!(initial.instance_id.as_deref(), Some("55aa"));
        assert_eq!(
            tracker.identity_source(),
            WindowIdentitySource::DesktopFileName
        );

        hypr.answer(window("0x77bb", "steam", "Steam", true));
        // Split a line across writes, and include an unrelated event.
        events
            .write_all(b"workspace>>2\nactivewindow>>steam,Steam\nactivewin")
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        events.write_all(b"dowv2>>77bb\n").unwrap();
        let steam = next(&mut tracker).unwrap().unwrap();
        assert_eq!(steam.app_id.as_deref(), Some("steam"));
        assert_eq!(
            tracker.identity_source(),
            WindowIdentitySource::ResourceClass
        );
    }

    #[test]
    fn an_empty_workspace_or_closed_window_clears_focus() {
        let hypr = FakeHyprland::start();
        hypr.answer(window("0x1", "foot", "", false));
        let (mut tracker, mut events) = hypr.connect(false);
        next(&mut tracker);
        events.write_all(b"activewindowv2>>\n").unwrap();
        assert_eq!(next(&mut tracker), Some(None));

        hypr.answer(window("0x2", "foot", "", false));
        events.write_all(b"activewindowv2>>2\n").unwrap();
        assert!(next(&mut tracker).unwrap().is_some());
        events
            .write_all(b"closewindow>>9\ncloseWindow>>2\nclosewindow>>2\n")
            .unwrap();
        assert_eq!(next(&mut tracker), Some(None));
    }

    #[test]
    fn title_changes_are_followed_only_when_titles_are_requested() {
        for titles in [false, true] {
            let hypr = FakeHyprland::start();
            hypr.answer(window("0x1", "foot", "one", false));
            let (mut tracker, mut events) = hypr.connect(titles);
            next(&mut tracker);
            if titles {
                hypr.answer(window("0x1", "foot", "two", false));
            }
            events.write_all(b"windowtitlev2>>1,two\n").unwrap();
            let update = tracker
                .next_window_timeout(Duration::from_millis(300))
                .unwrap();
            if titles {
                assert_eq!(update.unwrap().unwrap().title.as_deref(), Some("two"));
            } else {
                assert_eq!(update, None);
            }
        }
    }

    #[test]
    fn a_closed_event_socket_is_a_retryable_disconnect() {
        let hypr = FakeHyprland::start();
        hypr.answer(json!({}));
        let (mut tracker, events) = hypr.connect(false);
        assert_eq!(next(&mut tracker), Some(None), "nothing focused at start");
        drop(events);
        assert!(
            tracker
                .next_window_timeout(Duration::from_secs(2))
                .unwrap_err()
                .retryable
        );
    }

    #[test]
    fn addresses_are_normalized() {
        assert_eq!(normalize_address("0x55AA"), Some("55aa".into()));
        assert_eq!(normalize_address("55aa"), Some("55aa".into()));
        assert_eq!(normalize_address(","), None);
        assert_eq!(normalize_address(""), None);
    }
}
