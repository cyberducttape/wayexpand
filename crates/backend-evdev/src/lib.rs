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
//! - **Requires `input` group membership** (or an equivalent udev rule) to
//!   read `/dev/input/event*`, a broader grant than the Wayland sources
//!   need.
//!
//! Capture is deliberately **non-exclusive** (no `EVIOCGRAB`): the
//! compositor keeps delivering every key to the focused application
//! normally, exactly as if this source were not running. This source only
//! watches the same stream and, on a trigger match, asks the paired
//! `TextInjector` (typically `wayexpand-backend-libei`) to erase the
//! trigger and insert the replacement -- the same reactive model the other
//! sources use. It never takes over full keystroke pass-through the way an
//! exclusive Wayland input-method grab does, which keeps this source's
//! failure surface limited to "a match was missed," not "the user's typing
//! breaks."
//!
//! Kernel auto-repeat events are translated for matcher state while leaving
//! XKB's physical key state unchanged. The focused application still receives
//! the native repeat directly because capture remains non-exclusive.

mod device;

/// Return whether at least one actual keyboard event device is readable.
/// This is the same capability test used by [`EvdevSource::connect`], exposed
/// so setup and doctor do not mistake a readable mouse or touchpad node for a
/// usable keyboard.
pub fn readable_keyboard_available() -> bool {
    !device::discover_keyboards().keyboards.is_empty()
}

use std::{
    collections::{HashSet, VecDeque},
    os::fd::BorrowedFd,
    time::{Duration, Instant},
};

use thiserror::Error;
use wayexpand_backend_input_method::{key_action, key_action_and_update, key_chord, KeyAction};
use wayexpand_core::{InputEvent, InputSource, InputSourceError};
use wayland_client::protocol::wl_keyboard::KeyState;
use xkbcommon_rs::{Context, Keymap, State};

const SOURCE_NAME: &str = "evdev";
const POLL_TIMEOUT: Duration = Duration::from_millis(500);
// Allow the focused compositor/application to commit the non-exclusive
// physical keystrokes before the replacement backspaces are injected.
const DEFAULT_QUIET_PERIOD: Duration = Duration::from_millis(8);
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PENDING_EVENTS: usize = 4096;

#[derive(Debug, Error)]
pub enum EvdevError {
    #[error(
        "no keyboard device found under /dev/input; attach a keyboard, or if one is already \
         attached this may be a permission issue (see docs/SECURITY.md)"
    )]
    NoKeyboard,
    #[error(
        "no readable keyboard was found; {count} unreadable /dev/input/event* node(s) also exist, \
         and one or more may be a keyboard. Check input permissions and group membership (see \
         docs/SECURITY.md). If this still fails after logging out and back in, your systemd \
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
    #[error("timed out waiting for all keyboard keys to be released")]
    KeyReleaseTimeout,
}

impl EvdevError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            EvdevError::Poll(_) | EvdevError::Read { .. } | EvdevError::AllDevicesLost
        )
    }
}

pub struct EvdevSource {
    devices: Vec<device::KeyboardDevice>,
    state: State,
    pending: VecDeque<InputEvent>,
    /// Evdev keycodes currently held down. Capture is non-exclusive, so the
    /// application receives these presses too, and a match necessarily fires
    /// while the trigger's last key is still down. Injecting that same
    /// keycode then collides with the physical one -- see
    /// `wait_for_key_release`.
    pressed: HashSet<u32>,
    last_device_refresh: Instant,
}

impl EvdevSource {
    pub fn connect() -> Result<Self, EvdevError> {
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
            "evdev capture active: no per-field sensitive-content signal is available, so \
             matching is never suspended in password or other sensitive fields (docs/SECURITY.md)"
        );
        Ok(Self {
            devices: discovery.keyboards,
            state: State::new(keymap),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        })
    }

    /// One bounded poll of every open device. Returns without error (and
    /// without changing `self.pending`) if `timeout` elapses with nothing
    /// ready, so callers on the daemon's main loop can still service
    /// stop/pause/reload requests at a steady cadence even while idle.
    fn poll_once(&mut self, timeout: Duration) -> Result<(), EvdevError> {
        if self.devices.is_empty() {
            return Err(EvdevError::AllDevicesLost);
        }
        let timeout = rustix::event::Timespec {
            tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: timeout.subsec_nanos().into(),
        };
        let borrowed: Vec<BorrowedFd<'_>> = self
            .devices
            .iter()
            .map(device::KeyboardDevice::as_fd)
            .collect();
        let mut fds: Vec<rustix::event::PollFd<'_>> = borrowed
            .iter()
            .map(|fd| {
                rustix::event::PollFd::new(
                    fd,
                    rustix::event::PollFlags::IN
                        | rustix::event::PollFlags::ERR
                        | rustix::event::PollFlags::HUP
                        | rustix::event::PollFlags::NVAL,
                )
            })
            .collect();
        rustix::event::poll(&mut fds, Some(&timeout))
            .map_err(|error| EvdevError::Poll(error.to_string()))?;
        let mut ready = Vec::new();
        let mut lost = Vec::new();
        for (index, fd) in fds.iter().enumerate() {
            let revents = fd.revents();
            if revents.intersects(
                rustix::event::PollFlags::ERR
                    | rustix::event::PollFlags::HUP
                    | rustix::event::PollFlags::NVAL,
            ) {
                lost.push(index);
            } else if revents.contains(rustix::event::PollFlags::IN) {
                ready.push(index);
            }
        }
        drop(fds);
        drop(borrowed);
        self.drain_ready(&ready)?;
        let had_lost_devices = !lost.is_empty();
        for index in lost.into_iter().rev() {
            tracing::warn!(
                path = %self.devices[index].path().display(),
                "keyboard device disconnected"
            );
            self.devices.remove(index);
        }
        if had_lost_devices {
            self.reset_keyboard_state()?;
        }
        Ok(())
    }

    fn reset_keyboard_state(&mut self) -> Result<(), EvdevError> {
        let context = Context::new(0).map_err(|error| EvdevError::Keymap(error.to_string()))?;
        let keymap = Keymap::new_from_names(context, None, 0)
            .map_err(|error| EvdevError::Keymap(error.to_string()))?;
        self.state = State::new(keymap);
        self.pressed.clear();
        self.queue_event(InputEvent::Reset);
        tracing::warn!("resetting evdev keyboard state after device disconnect");
        Ok(())
    }

    /// Keep translated input bounded while the daemon is waiting for a safe
    /// replacement point. Dropping only the newest event could leave the
    /// matcher/application streams out of sync, so overflow abandons the
    /// pending transaction and emits one reset boundary instead.
    fn queue_event(&mut self, event: InputEvent) {
        if self.pending.len() >= MAX_PENDING_EVENTS {
            self.pending.clear();
            self.pending.push_back(InputEvent::Reset);
            tracing::warn!(
                maximum = MAX_PENDING_EVENTS,
                "evdev input queue overflow; resetting matcher state"
            );
            return;
        }
        self.pending.push_back(event);
    }

    fn refresh_devices(&mut self) {
        let discovery = device::discover_keyboards();
        for keyboard in discovery.keyboards {
            if self
                .devices
                .iter()
                .any(|existing| existing.path() == keyboard.path())
            {
                continue;
            }
            tracing::info!(path = %keyboard.path().display(), "keyboard device connected");
            self.devices.push(keyboard);
        }
    }

    fn drain_ready(&mut self, ready_indices: &[usize]) -> Result<(), EvdevError> {
        for &index in ready_indices {
            let events = {
                let device = &mut self.devices[index];
                device.fetch_events().map_err(|error| EvdevError::Read {
                    path: device.path().display().to_string(),
                    message: error.to_string(),
                })?
            };
            for event in events {
                self.translate(event);
            }
        }
        Ok(())
    }

    /// Translates one raw evdev event into zero or more `InputEvent`s,
    /// pushed directly onto `self.pending`. A key press can yield both a
    /// hotkey chord and an ordinary matcher event (mirrors how
    /// `backend-input-method` reports both from the same key press).
    fn translate(&mut self, event: evdev::InputEvent) {
        let evdev::EventSummary::Key(_, key_code, value) = event.destructure() else {
            return;
        };
        let keycode = u32::from(key_code.code());
        if value == 2 {
            let action = key_action(&self.state, keycode);
            self.queue_action(action);
            return;
        }
        let key_state = match value {
            0 => KeyState::Released,
            1 => KeyState::Pressed,
            _ => return,
        };
        match key_state {
            KeyState::Pressed => {
                self.pressed.insert(keycode);
            }
            _ => {
                self.pressed.remove(&keycode);
            }
        }
        if key_state == KeyState::Pressed {
            if let Some(chord) = key_chord(&self.state, keycode) {
                self.queue_event(InputEvent::Key(chord));
            }
        }
        let action = key_action_and_update(&mut self.state, keycode, key_state);
        self.queue_action(action);
    }

    fn queue_action(&mut self, action: Option<KeyAction>) {
        let translated = match action {
            Some(KeyAction::Delete) => Some(InputEvent::Backspace),
            // Commit carries "\n"/"\t" for the app, which already received
            // the real key natively; the matcher only needs the boundary.
            Some(KeyAction::Commit(text)) => text.chars().next().map(InputEvent::Delimiter),
            Some(KeyAction::Text(text)) => Some(InputEvent::Text(text)),
            Some(KeyAction::Ignore) | None => None,
            Some(KeyAction::Unsupported) => Some(InputEvent::Reset),
        };
        if let Some(event) = translated {
            self.queue_event(event);
        }
    }

    /// Whether any key is physically held right now.
    pub fn keys_held(&self) -> bool {
        !self.pressed.is_empty()
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
            self.poll_once(remaining)
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
            self.poll_once(remaining.min(POLL_TIMEOUT))
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
        // from latency-sensitive polling. Existing descriptors remain active;
        // this periodic pass is only the fallback for keyboards added after
        // startup. Release and quiet-period safety waits never refresh.
        if self.last_device_refresh.elapsed() >= DEVICE_REFRESH_INTERVAL {
            self.refresh_devices();
            self.last_device_refresh = Instant::now();
        }
        self.poll_once(timeout).map_err(|error| InputSourceError {
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
}

impl InputSource for EvdevSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    fn next_event(&mut self) -> Result<InputEvent, InputSourceError> {
        loop {
            if let Some(event) = self.next_event_timeout(POLL_TIMEOUT)? {
                return Ok(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> State {
        let keymap = Keymap::new_from_names(Context::new(0).unwrap(), None, 0).unwrap();
        State::new(keymap)
    }

    /// A press on a resolvable key always yields a candidate hotkey chord
    /// first (mirroring `backend-input-method`; the engine ignores chords
    /// that are not configured as a hotkey) followed by the ordinary
    /// matcher action. Returns just the latter, which is what these tests
    /// care about.
    fn press(state: &mut State, code: u16) -> Option<InputEvent> {
        let event = evdev::InputEvent::new(evdev::EventType::KEY.0, code, 1);
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        source.translate(event);
        *state = source.state;
        assert!(
            matches!(source.pending.pop_front(), Some(InputEvent::Key(_))),
            "expected a candidate hotkey chord to be queued first"
        );
        source.pending.pop_front()
    }

    #[test]
    fn printable_key_becomes_text_event() {
        let mut state = test_state();
        // KEY_A = 30 in linux/input-event-codes.h.
        assert_eq!(press(&mut state, 30), Some(InputEvent::Text("a".into())));
    }

    #[test]
    fn backspace_key_becomes_backspace_event() {
        let mut state = test_state();
        // KEY_BACKSPACE = 14.
        assert_eq!(press(&mut state, 14), Some(InputEvent::Backspace));
    }

    #[test]
    fn enter_key_preserves_delimiter_event() {
        let mut state = test_state();
        // KEY_ENTER = 28.
        assert_eq!(press(&mut state, 28), Some(InputEvent::Delimiter('\n')));
    }

    #[test]
    fn auto_repeat_forwards_text_without_a_hotkey_event() {
        let mut state = test_state();
        let event = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 2);
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(&mut state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        source.translate(event);
        assert_eq!(
            source.pending.pop_front(),
            Some(InputEvent::Text("a".into()))
        );
        assert_eq!(source.pending.pop_front(), None);
    }

    #[test]
    fn release_events_do_not_emit_text() {
        let mut state = test_state();
        let event = evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0);
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(&mut state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        source.translate(event);
        assert_eq!(source.pending.pop_front(), None);
    }

    #[test]
    fn held_keys_are_tracked_until_released() {
        let mut state = test_state();
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(&mut state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        assert!(!source.keys_held());

        // KEY_A = 30, KEY_B = 48. Overlapping presses, as a fast typist
        // produces, must all be seen as held: injecting while any of them is
        // down is what ate characters from a replacement.
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
        assert!(source.keys_held());
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 48, 1));
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 0));
        assert!(source.keys_held(), "the second key is still down");
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 48, 0));
        assert!(!source.keys_held());
    }

    #[test]
    fn quiet_period_rejects_already_queued_input() {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            pending: VecDeque::from([InputEvent::Text("a".into())]),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };

        assert!(!source
            .wait_for_input_quiet(Duration::from_millis(40))
            .unwrap());
    }

    #[test]
    fn auto_repeat_does_not_clear_the_held_key() {
        let mut state = test_state();
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(&mut state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
        // Value 2 is kernel auto-repeat. It must not be mistaken for a release.
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 2));
        assert!(source.keys_held());
    }

    #[test]
    fn disconnect_resets_held_keys_and_modifier_state() {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 29, 1));
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
        assert!(source.keys_held());
        source.reset_keyboard_state().unwrap();
        assert!(!source.keys_held());
        assert_eq!(source.pending.pop_back(), Some(InputEvent::Reset));
        source.pending.clear();

        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
        assert!(matches!(
            source.pending.pop_front(),
            Some(InputEvent::Key(_))
        ));
        assert_eq!(
            source.pending.pop_front(),
            Some(InputEvent::Text("a".into()))
        );
    }

    #[test]
    fn held_key_at_release_deadline_is_rejected() {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            pending: VecDeque::new(),
            pressed: HashSet::from([30]),
            last_device_refresh: Instant::now(),
        };
        let error = source.wait_for_key_release(Duration::ZERO).unwrap_err();
        assert!(error.message.contains("timed out"));
    }

    #[test]
    fn empty_device_set_is_not_treated_as_a_successful_poll() {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };
        assert!(matches!(
            source.poll_once(Duration::ZERO),
            Err(EvdevError::AllDevicesLost)
        ));
    }

    #[test]
    fn pending_event_overflow_resets_the_queue() {
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: test_state(),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
            last_device_refresh: Instant::now(),
        };

        for _ in 0..=MAX_PENDING_EVENTS {
            source.queue_event(InputEvent::Text("a".into()));
        }

        assert_eq!(source.pending.len(), 1);
        assert_eq!(source.pending.front(), Some(&InputEvent::Reset));
    }
}
