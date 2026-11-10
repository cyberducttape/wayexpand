//! Compositor-agnostic keyboard capture via raw evdev devices.
//!
//! The other sources (`wayexpand-backend-input-method`) rely on Wayland
//! protocols (`zwp_input_method_manager_v2`) that not every compositor
//! advertises -- notably KWin/KDE Plasma as of KWin 6.6. This source reads
//! keyboard input directly from the kernel (`/dev/input/eventN`), so capture
//! is compositor-independent. Output still requires a compatible libei/EIS
//! portal or virtual-keyboard protocol, at a real cost the Wayland sources do not have:
//!
//! - **No sensitive-field signal.** Wayland's input-method protocol tells a
//!   backend when the focused field is a password/sensitive field; raw
//!   evdev has no concept of "which application field is focused" at all.
//!   This source therefore never emits
//!   `InputEvent::FocusChanged { sensitive: true }` -- matching is never
//!   suspended in password fields. Deploy it only where that tradeoff is
//!   acceptable, and see `docs/SECURITY.md`.
//! - **No active-layout provider.** The evdev source refuses to translate
//!   keys for matching or hotkeys while the compositor layout is unknown.
//!   Native typing remains untouched, and physical key state is still tracked
//!   for output interruption safety.
//! - **Requires explicit `/dev/input` authorization.** The installer defaults
//!   to active-seat logind/uaccess ACLs; `--access=input-group` remains an
//!   explicitly selected legacy fallback with broader permanent visibility.
//!
//! Capture is deliberately **non-exclusive** (no `EVIOCGRAB`): the
//! compositor keeps delivering every key to the focused application
//! normally, exactly as if this source were not running. This source only
//! watches the same stream and, on a trigger match, asks the paired
//! `TextInjector` (typically `wayexpand-backend-libei`) to erase the
//! trigger and insert the replacement -- the same reactive model the other
//! sources use. Non-exclusive capture preserves normal physical-key
//! delivery; it does not make replacement atomic with that delivery. The
//! application can process the trigger, change focus, or otherwise move its
//! cursor before synthetic erase and insertion events arrive. A partial or
//! misdirected replacement can therefore alter unintended text, not just
//! miss a match. Treat evdev replacement as experimental best effort and use
//! the route assurance reported by the daemon; see `docs/BACKENDS.md` and
//! `docs/SECURITY.md`.
//!
//! Kernel auto-repeat events are translated for matcher state while leaving
//! XKB's physical key state unchanged. The focused application still receives
//! the native repeat directly because capture remains non-exclusive.

mod device;
mod poll;
mod translation;

/// Return whether at least one actual keyboard event device is readable.
/// This is the same capability test used by [`EvdevSource::connect`], exposed
/// so setup and doctor do not mistake a readable mouse or touchpad node for a
/// usable keyboard.
pub fn readable_keyboard_available() -> bool {
    !device::discover_keyboards().keyboards.is_empty()
}

use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use thiserror::Error;
use wayexpand_core::{InputEvent, InputSource, InputSourceCapabilities, InputSourceError};
use xkbcommon_rs::{xkb_state::KeyDirection, Context, Keymap, State};

const SOURCE_NAME: &str = "evdev";
const POLL_TIMEOUT: Duration = Duration::from_millis(500);
// Allow the focused compositor/application to commit the non-exclusive
// physical keystrokes before the replacement backspaces are injected.
const DEFAULT_QUIET_PERIOD: Duration = Duration::from_millis(8);
// Hotplug discovery is intentionally bounded and separate from event polling,
// but a 30-second window is too surprising for a compatibility fallback.
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_PENDING_EVENTS: usize = 4096;
/// Bound the raw-event translation work performed during one input turn. The
/// kernel ring buffer and device adapter retain any remaining events for the
/// next turn; no event batch is silently truncated.
const MAX_RAW_EVENTS_PER_SLICE: usize = 128;

#[derive(Default)]
struct PollBuffers {
    descriptors: Vec<libc::pollfd>,
    ready: Vec<usize>,
    lost: Vec<usize>,
}

thread_local! {
    /// `poll_once` runs on the source's owning reactor thread. Reuse its
    /// descriptor and index storage instead of allocating three vectors on
    /// every idle/input turn.
    static POLL_BUFFERS: RefCell<PollBuffers> = RefCell::new(PollBuffers::default());
}

#[cfg(feature = "benchmark")]
pub mod benchmark_support;

/// A compositor supplied keymap snapshot. `generation` must change whenever
/// the effective layout changes, including while modifiers are held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLayoutSnapshot {
    pub generation: u64,
    /// Active XKB layout group as reported by the compositor.
    pub group: u32,
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
}

/// Supplies the active compositor keymap to raw evdev capture. Returning
/// `None` means the active layout cannot be established and matching must
/// fail closed.
pub trait KeyboardLayoutProvider: Send {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot>;
}

struct UnavailableLayoutProvider;

impl KeyboardLayoutProvider for UnavailableLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        None
    }
}

static INPUT_QUEUE_OVERFLOWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How often, since process start, translated input outgrew its queue and
/// was dropped (the matcher is reset each time). For the daemon status.
pub fn input_queue_overflow_total() -> u64 {
    INPUT_QUEUE_OVERFLOWS.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Error)]
pub enum EvdevError {
    #[error(
        "no keyboard device found under /dev/input; attach a keyboard, or if one is already \
         attached this may be a permission issue (see docs/SECURITY.md)"
    )]
    NoKeyboard,
    #[error(
        "no readable keyboard was found; {count} unreadable /dev/input/event* node(s) also exist, \
         and one or more may be a keyboard. Check active-seat /dev/input ACLs, or the explicit \
         legacy input-group fallback (see docs/SECURITY.md). If this still fails after logging \
         out and back in, your systemd \
         --user manager may still have the old group list -- run `loginctl terminate-user $USER` \
         (ends all your sessions) or reboot, then retry"
    )]
    PermissionDenied { count: usize },
    #[error("could not build a keymap for the system keyboard layout: {0}")]
    Keymap(String),
    #[error("polling input devices failed: {0}")]
    Poll(String),
    #[error("reading input device {path} failed: {message}")]
    Read { path: String, message: String },
    #[error("all keyboard devices were disconnected")]
    AllDevicesLost,
    #[error("watching /dev/input for keyboard hotplug failed: {0}")]
    HotplugMonitor(String),
    #[error("timed out waiting for all keyboard keys to be released")]
    KeyReleaseTimeout,
}

impl EvdevError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            EvdevError::Poll(_)
                | EvdevError::Read { .. }
                | EvdevError::AllDevicesLost
                | EvdevError::HotplugMonitor(_)
        )
    }
}

pub struct EvdevSource {
    devices: Vec<Box<dyn device::KeyboardDeviceAccess>>,
    state: State,
    layout_provider: Box<dyn KeyboardLayoutProvider>,
    layout_generation: Option<u64>,
    layout_available: bool,
    raw_key_events_total: u64,
    layout_unavailable_key_events_total: u64,
    pending: VecDeque<InputEvent>,
    /// Reused storage for one device's raw batch. The cursor lets a large
    /// batch be translated over multiple scheduling slices without copying
    /// event records into a fresh Vec on every poll.
    raw_events: Vec<evdev::InputEvent>,
    raw_event_cursor: usize,
    raw_event_device: Option<usize>,
    pending_lost_devices: Vec<usize>,
    /// Evdev keycodes currently held down. Capture is non-exclusive, so the
    /// application receives these presses too, and a match necessarily fires
    /// while the trigger's last key is still down. Injecting that same
    /// keycode then collides with the physical one -- see
    /// `wait_for_key_release`.
    /// Keycodes held on each active device slot. Device identity is its
    /// current integer slot, so input batches do not clone device paths.
    /// Slots are changed only after buffered events have been consumed.
    pressed: Vec<HashSet<u32>>,
    last_device_refresh: Instant,
    /// Optional non-blocking eventfd that ends an idle wait early; see
    /// [`EvdevSource::set_wake_fd`].
    wake: Option<Arc<OwnedFd>>,
    /// Watches device-node creation/removal so an output transaction can
    /// fail closed if a keyboard is hotplugged while only the current device
    /// descriptors are being monitored.
    topology_watch: Option<OwnedFd>,
    /// Defer device discovery until the next ordinary input turn, so handling
    /// a hotplug notification never delays cancellation of in-flight output.
    topology_refresh_pending: bool,
}

impl EvdevSource {
    /// Content-free capture diagnostics. These counters distinguish a quiet
    /// device from keys received by evdev but refused because no trustworthy
    /// active keyboard layout was available.
    pub fn input_diagnostics(&self) -> EvdevInputDiagnostics {
        EvdevInputDiagnostics {
            raw_key_events_total: self.raw_key_events_total,
            layout_unavailable_key_events_total: self.layout_unavailable_key_events_total,
        }
    }

    pub fn connect() -> Result<Self, EvdevError> {
        Self::connect_with_layout_provider(Box::new(UnavailableLayoutProvider))
    }

    /// Connect using a compositor-specific layout source. Compositors without
    /// a trustworthy active-keymap API use the unavailable provider, which
    /// leaves native key delivery alone but disables evdev matching.
    pub fn connect_with_layout_provider(
        layout_provider: Box<dyn KeyboardLayoutProvider>,
    ) -> Result<Self, EvdevError> {
        // Install the topology watch before discovery. If a keyboard appears
        // during discovery, either the scan sees it or inotify reports it.
        let topology_watch = create_topology_watch(Path::new("/dev/input"))
            .map_err(|error| EvdevError::HotplugMonitor(error.to_string()))?;
        let discovery = device::discover_keyboards();
        if discovery.keyboards.is_empty() {
            return Err(if discovery.unreadable_event_paths.is_empty() {
                EvdevError::NoKeyboard
            } else {
                EvdevError::PermissionDenied {
                    count: discovery.unreadable_event_paths.len(),
                }
            });
        }
        let context = Context::new(0).map_err(|error| EvdevError::Keymap(error.to_string()))?;
        let keymap = Keymap::new_from_names(context, None, 0)
            .map_err(|error| EvdevError::Keymap(error.to_string()))?;
        tracing::warn!(
            "evdev capture active without per-field sensitive-content protection; matching is \
             never suspended in password or other sensitive fields (docs/SECURITY.md)"
        );
        let devices: Vec<Box<dyn device::KeyboardDeviceAccess>> = discovery
            .keyboards
            .into_iter()
            .map(|keyboard| Box::new(keyboard) as Box<dyn device::KeyboardDeviceAccess>)
            .collect();
        let pressed = (0..devices.len()).map(|_| HashSet::new()).collect();
        let mut source = Self {
            devices,
            state: State::new(keymap),
            layout_provider,
            layout_generation: None,
            layout_available: false,
            raw_key_events_total: 0,
            layout_unavailable_key_events_total: 0,
            pending: VecDeque::new(),
            raw_events: Vec::new(),
            raw_event_cursor: 0,
            raw_event_device: None,
            pending_lost_devices: Vec::new(),
            pressed,
            last_device_refresh: Instant::now(),
            wake: None,
            topology_watch: Some(topology_watch),
            topology_refresh_pending: false,
        };
        if !source.refresh_layout() {
            tracing::warn!(
                "no trustworthy compositor keyboard-layout snapshot is available; evdev \
                 matching and hotkeys are disabled until one becomes available"
            );
        }
        Ok(source)
    }

    /// Install a non-blocking eventfd that ends [`Self::next_event_timeout`]
    /// early when another thread has work for the caller. The source drains
    /// it when it fires. Key-release and quiet-period safety waits ignore it.
    pub fn set_wake_fd(&mut self, wake: Option<Arc<OwnedFd>>) {
        self.wake = wake;
    }

    /// One bounded poll of every open device. Returns without error (and
    /// without changing `self.pending`) if `timeout` elapses with nothing
    /// ready, so callers on the daemon's main loop can still service
    /// stop/pause/reload requests at a steady cadence even while idle.
    /// Whether any key is physically held right now.
    pub fn keys_held(&self) -> bool {
        self.pressed.iter().any(|keys| !keys.is_empty())
    }

    pub fn has_pending_events(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Remove translated events that arrived while a replacement was waiting
    /// for the physical trigger key to be released. Callers must validate the
    /// events before applying a replacement at the moved cursor.
    pub fn take_pending_events(&mut self) -> Vec<InputEvent> {
        self.pending.drain(..).collect()
    }

    /// Waits for a short interval with no newly received input. Returns
    /// `false` if another event arrived during that interval. Non-exclusive
    /// capture cannot prevent the focused application from receiving those
    /// events, so callers should abandon an expansion when this returns false
    /// rather than modifying a moving cursor.
    pub fn wait_for_input_quiet(&mut self, timeout: Duration) -> Result<bool, InputSourceError> {
        if self.has_pending_events() {
            return Ok(false);
        }
        let deadline = Instant::now() + timeout;
        let quiet_deadline = Instant::now() + DEFAULT_QUIET_PERIOD;
        while Instant::now() < quiet_deadline {
            let remaining = quiet_deadline
                .saturating_duration_since(Instant::now())
                .min(deadline.saturating_duration_since(Instant::now()));
            if remaining.is_zero() {
                break;
            }
            self.poll_once(remaining, false)
                .map_err(|error| InputSourceError {
                    source: SOURCE_NAME,
                    retryable: error.is_retryable(),
                    message: error.to_string(),
                })?;
            if self.has_pending_events() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Blocks until every physically held key has been released. A timeout is
    /// an unsafe state, not successful completion: callers must abandon the
    /// expansion rather than injecting while a physical key may still be held.
    ///
    /// Callers must do this before injecting a replacement. A match fires on
    /// key-down, so the trigger's last key is still held at that moment;
    /// injecting the same keycode while the compositor already considers it
    /// pressed makes the duplicate press read as auto-repeat and the
    /// matching release cancel the physical one, silently eating exactly
    /// those characters from the replacement.
    ///
    /// Events that arrive while waiting are queued as usual, so nothing is
    /// dropped and ordering is preserved; they are simply processed after
    /// the expansion.
    pub fn wait_for_key_release(&mut self, timeout: Duration) -> Result<(), InputSourceError> {
        let deadline = Instant::now() + timeout;
        while self.keys_held() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(InputSourceError {
                    source: SOURCE_NAME,
                    retryable: false,
                    message: EvdevError::KeyReleaseTimeout.to_string(),
                });
            }
            self.poll_once(remaining.min(POLL_TIMEOUT), false)
                .map_err(|error| InputSourceError {
                    source: SOURCE_NAME,
                    retryable: error.is_retryable(),
                    message: error.to_string(),
                })?;
            if self
                .pending
                .iter()
                .any(|event| matches!(event, InputEvent::Reset))
            {
                return Err(InputSourceError {
                    source: SOURCE_NAME,
                    retryable: true,
                    message: "keyboard state was reset while waiting for release".into(),
                });
            }
        }
        Ok(())
    }

    /// Bounded-wait event fetch used by the daemon's main loop instead of
    /// the `InputSource` trait's blocking `next_event`, mirroring
    /// `InputMethodSource::next_event_timeout`: `Ok(None)` on a plain
    /// timeout (nothing ready), so the caller can still service
    /// stop/pause/reload requests at a steady cadence while idle.
    pub fn next_event_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InputEvent>, InputSourceError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        // Device discovery scans /dev/input and is intentionally decoupled
        // from latency-sensitive polling. A topology event forces this pass
        // on the next ordinary turn; during output, the event is returned to
        // the daemon immediately so it can cancel before discovery begins.
        if !self.has_buffered_raw_events()
            && self.pending_lost_devices.is_empty()
            && (self.topology_refresh_pending
                || self.last_device_refresh.elapsed() >= DEVICE_REFRESH_INTERVAL)
        {
            self.refresh_devices();
            self.topology_refresh_pending = false;
            self.last_device_refresh = Instant::now();
        }
        self.poll_once(timeout, true)
            .map_err(|error| InputSourceError {
                source: SOURCE_NAME,
                retryable: error.is_retryable(),
                message: error.to_string(),
            })?;
        if self.devices.is_empty() {
            return Err(InputSourceError {
                source: SOURCE_NAME,
                retryable: true,
                message: EvdevError::AllDevicesLost.to_string(),
            });
        }
        Ok(self.pending.pop_front())
    }

    /// Poll open keyboard devices and the input-device topology without
    /// sleeping. Reports raw key activity, including modifiers, and device
    /// changes so the daemon can interrupt non-atomic output before unseen
    /// input changes its target.
    pub fn next_event_during_output(
        &mut self,
        timeout: Duration,
    ) -> Result<bool, InputSourceError> {
        if self.pending.pop_front().is_some() {
            // A translated pending event is necessarily backed by physical
            // input and therefore interrupts an in-flight replacement.
            return Ok(true);
        }
        if self
            .poll_topology_nonblocking()
            .map_err(|error| InputSourceError {
                source: SOURCE_NAME,
                retryable: error.is_retryable(),
                message: error.to_string(),
            })?
        {
            return Ok(true);
        }
        if self.has_buffered_raw_events() {
            let (key_activity, _) = self.process_buffered_raw_events(MAX_RAW_EVENTS_PER_SLICE);
            if key_activity {
                self.pending.pop_front();
                return Ok(true);
            }
            return Ok(self.raw_events[self.raw_event_cursor..]
                .iter()
                .any(|event| matches!(event.destructure(), evdev::EventSummary::Key(..))));
        }
        let mut key_activity = self
            .poll_once(timeout, true)
            .map_err(|error| InputSourceError {
                source: SOURCE_NAME,
                retryable: error.is_retryable(),
                message: error.to_string(),
            })?;
        key_activity |= self.raw_events[self.raw_event_cursor..]
            .iter()
            .any(|event| matches!(event.destructure(), evdev::EventSummary::Key(..)));
        if self.devices.is_empty() {
            return Err(InputSourceError {
                source: SOURCE_NAME,
                retryable: true,
                message: EvdevError::AllDevicesLost.to_string(),
            });
        }
        if self.topology_watch.is_none() && !key_activity {
            return Err(InputSourceError {
                source: SOURCE_NAME,
                retryable: true,
                message: "keyboard hotplug monitoring is unavailable during non-atomic output"
                    .into(),
            });
        }
        if key_activity {
            // Match the old monitor contract: consume one translated event
            // because the caller uses this method only as an interruption
            // signal; it must not later replay that event as matcher input.
            self.pending.pop_front();
        }
        Ok(key_activity)
    }

    /// Check the topology watch even when a previously read device batch is
    /// still being translated. Output cancellation must not wait for a large
    /// raw-event backlog to drain before noticing a keyboard hotplug.
    fn poll_topology_nonblocking(&mut self) -> Result<bool, EvdevError> {
        let Some(watch) = self.topology_watch.as_ref() else {
            return Ok(false);
        };
        let mut fds = [rustix::event::PollFd::new(
            watch,
            rustix::event::PollFlags::IN
                | rustix::event::PollFlags::ERR
                | rustix::event::PollFlags::HUP
                | rustix::event::PollFlags::NVAL,
        )];
        let ready = rustix::event::poll(
            &mut fds,
            Some(&rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .map_err(|error| EvdevError::HotplugMonitor(error.to_string()))?;
        if ready == 0 {
            return Ok(false);
        }
        let revents = fds[0].revents();
        if revents.intersects(
            rustix::event::PollFlags::ERR
                | rustix::event::PollFlags::HUP
                | rustix::event::PollFlags::NVAL,
        ) {
            return Err(EvdevError::HotplugMonitor(
                "inotify descriptor became unavailable".into(),
            ));
        }
        if !revents.contains(rustix::event::PollFlags::IN) {
            return Ok(false);
        }
        let changed = drain_topology_events(watch.as_fd())?;
        self.topology_refresh_pending |= changed;
        Ok(changed)
    }
}

/// Aggregate evdev telemetry; never contains keycodes or typed content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvdevInputDiagnostics {
    pub raw_key_events_total: u64,
    pub layout_unavailable_key_events_total: u64,
}

impl InputSource for EvdevSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    fn capabilities(&self) -> InputSourceCapabilities {
        InputSourceCapabilities {
            layout_aware: self.layout_available,
            ..InputSourceCapabilities::EVDEV
        }
    }

    fn next_event(&mut self) -> Result<InputEvent, InputSourceError> {
        loop {
            if let Some(event) = self.next_event_timeout(POLL_TIMEOUT)? {
                return Ok(event);
            }
        }
    }
}

/// Drain a nonblocking eventfd or pipe after it woke a poll. Eventfd requires
/// reads of exactly eight bytes, so pipe bursts are consumed in a loop too.
fn drain_wake_fd(fd: BorrowedFd<'_>) {
    let mut buffer = [0_u8; 8];
    for _ in 0..64 {
        match rustix::io::read(fd, &mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => break,
            Err(_) => break,
        }
    }
}

fn create_topology_watch(path: &Path) -> rustix::io::Result<OwnedFd> {
    use rustix::fs::inotify::{self, CreateFlags, WatchFlags};

    let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
    inotify::add_watch(
        &fd,
        path,
        WatchFlags::CREATE
            | WatchFlags::ATTRIB
            | WatchFlags::DELETE
            | WatchFlags::MOVED_FROM
            | WatchFlags::MOVED_TO
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF,
    )?;
    Ok(fd)
}

/// Consume pending device-topology notifications. Any event, including queue
/// overflow, means the monitor cannot prove that all physical keyboards were
/// observed during a replacement, so callers interrupt that operation.
fn drain_topology_events(fd: BorrowedFd<'_>) -> Result<bool, EvdevError> {
    use rustix::fs::inotify::{self, ReadFlags};

    let mut changed = false;
    let mut buffer = [std::mem::MaybeUninit::uninit(); 4096];
    let mut reader = inotify::Reader::new(fd, &mut buffer);
    let mut remaining = 512;
    loop {
        match reader.next() {
            Ok(event) => {
                if remaining == 0 {
                    return Err(EvdevError::HotplugMonitor(
                        "input-device topology event queue exceeded its drain budget".into(),
                    ));
                }
                remaining -= 1;
                let flags = event.events();
                if flags.intersects(ReadFlags::IGNORED | ReadFlags::UNMOUNT) {
                    return Err(EvdevError::HotplugMonitor(
                        "input-device directory watch was removed".into(),
                    ));
                }
                changed = true;
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => break,
            Err(error) => return Err(EvdevError::HotplugMonitor(error.to_string())),
        }
    }
    Ok(changed)
}

/// Poll the device descriptors and, when present, the final wake descriptor.
/// Keeping wake handling here lets its readiness and drain behavior be tested
/// without opening a host keyboard device.
#[cfg(any(test, feature = "benchmark"))]
fn poll_with_wake(
    fds: &mut [rustix::event::PollFd<'_>],
    timeout: Duration,
    wake: Option<&OwnedFd>,
) -> Result<bool, EvdevError> {
    let timeout = rustix::event::Timespec {
        tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: timeout.subsec_nanos().into(),
    };
    match rustix::event::poll(fds, Some(&timeout)) {
        Ok(_) => {}
        // A handled signal (the daemon's SIGTERM/SIGINT handler) interrupts
        // poll, which is never restarted. That is not a device failure:
        // reporting it as one made the daemon reopen every keyboard and
        // reset key state. Report no readiness and let the caller poll again.
        Err(rustix::io::Errno::INTR) => {
            for fd in fds.iter_mut() {
                fd.clear_revents();
            }
            return Ok(false);
        }
        Err(error) => return Err(EvdevError::Poll(error.to_string())),
    }
    let woken = wake.is_some()
        && fds
            .last()
            .is_some_and(|fd| fd.revents().contains(rustix::event::PollFlags::IN));
    if woken {
        if let Some(wake) = wake {
            drain_wake_fd(wake.as_fd());
        }
    }
    Ok(woken)
}

#[cfg(test)]
mod tests;
