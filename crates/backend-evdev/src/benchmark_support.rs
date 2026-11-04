/// Synthetic device harness for the full capture-and-translation benchmark.
/// It exercises descriptor polling, device batch reads, XKB translation and
/// event queuing without requiring access to a physical keyboard.
use super::*;
use std::{
    io::{Read, Write},
    os::fd::BorrowedFd,
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex},
};

struct BenchKeyboard {
    reader: UnixStream,
    events: Arc<Mutex<Vec<evdev::InputEvent>>>,
}

impl device::KeyboardDeviceAccess for BenchKeyboard {
    fn path(&self) -> &Path {
        Path::new("/benchmark/evdev-keyboard")
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }

    fn fetch_events(&mut self, output: &mut Vec<evdev::InputEvent>) -> std::io::Result<()> {
        let mut ready = [0_u8; 1];
        let _ = self.reader.read(&mut ready)?;
        output.clear();
        output.extend(
            self.events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .drain(..),
        );
        Ok(())
    }
}

struct BenchLayoutProvider;

impl KeyboardLayoutProvider for BenchLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        Some(KeyboardLayoutSnapshot {
            generation: 1,
            group: 0,
            rules: None,
            model: None,
            layout: None,
            variant: None,
            options: None,
        })
    }
}

pub struct CaptureSource {
    source: EvdevSource,
    writer: UnixStream,
    events: Arc<Mutex<Vec<evdev::InputEvent>>>,
}

impl CaptureSource {
    pub fn new() -> Self {
        let (reader, writer) = UnixStream::pair().expect("create benchmark device");
        reader
            .set_nonblocking(true)
            .expect("set benchmark device nonblocking");
        let events = Arc::new(Mutex::new(Vec::with_capacity(2)));
        let context = Context::new(0).expect("create benchmark XKB context");
        let keymap = Keymap::new_from_names(context, None, 0).expect("create benchmark keymap");
        Self {
            source: EvdevSource {
                devices: vec![Box::new(BenchKeyboard {
                    reader,
                    events: Arc::clone(&events),
                })],
                state: State::new(keymap),
                layout_provider: Box::new(BenchLayoutProvider),
                layout_generation: Some(1),
                layout_available: true,
                pending: VecDeque::with_capacity(4),
                raw_events: Vec::with_capacity(4),
                raw_event_cursor: 0,
                raw_event_device: None,
                pending_lost_devices: Vec::new(),
                pressed: vec![HashSet::new()],
                last_device_refresh: Instant::now(),
                wake: None,
                topology_watch: None,
                topology_refresh_pending: false,
            },
            writer,
            events,
        }
    }

    /// Capture and translate one ASCII key press/release pair; return the
    /// number of generated core input events.
    pub fn capture_ascii_pair(&mut self, keycode: u16) -> usize {
        self.capture_ascii_pair_with(keycode, false)
    }

    pub fn capture_ascii_pair_allocating_baseline(&mut self, keycode: u16) -> usize {
        self.capture_ascii_pair_with(keycode, true)
    }

    fn capture_ascii_pair_with(&mut self, keycode: u16, allocating_baseline: bool) -> usize {
        {
            let mut events = self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            events.clear();
            events.push(evdev::InputEvent::new(evdev::EventType::KEY.0, keycode, 1));
            events.push(evdev::InputEvent::new(evdev::EventType::KEY.0, keycode, 0));
        }
        self.writer.write_all(&[1]).expect("wake benchmark device");
        if allocating_baseline {
            self.source
                .poll_once_allocating(Duration::from_millis(1), false)
                .expect("poll and translate benchmark device");
        } else {
            self.source
                .poll_once(Duration::from_millis(1), false)
                .expect("poll and translate benchmark device");
        }
        let count = self.source.pending.len();
        self.source.pending.clear();
        count
    }
}

impl Default for CaptureSource {
    fn default() -> Self {
        Self::new()
    }
}
