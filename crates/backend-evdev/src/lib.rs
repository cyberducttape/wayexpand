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

#[derive(Debug, Error)]
pub enum EvdevError {
    #[error(
        "no keyboard device found under /dev/input; attach a keyboard, or if one is already \
         attached this may be a permission issue (see docs/SECURITY.md)"
    )]
    NoKeyboard,
    #[error(
        "{count} keyboard device(s) exist under /dev/input but none are readable by this user; \
         add your user to the `input` group and log in again (see docs/SECURITY.md). If this \
         still fails after logging out and back in, your systemd --user manager likely did not \
         restart and is still running with your old group list -- run `loginctl terminate-user \
         $USER` (ends all your sessions) or reboot, then retry"
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
}

impl EvdevSource {
    pub fn connect() -> Result<Self, EvdevError> {
        let discovery = device::discover_keyboards();
        if discovery.keyboards.is_empty() {
            return Err(if discovery.permission_denied_paths.is_empty() {
                EvdevError::NoKeyboard
            } else {
                EvdevError::PermissionDenied {
                    count: discovery.permission_denied_paths.len(),
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
        })
    }

    /// One bounded poll of every open device. Returns without error (and
    /// without changing `self.pending`) if `timeout` elapses with nothing
    /// ready, so callers on the daemon's main loop can still service
    /// stop/pause/reload requests at a steady cadence even while idle.
    fn poll_once(&mut self, timeout: Duration) -> Result<(), EvdevError> {
        if self.devices.is_empty() {
            return Ok(());
        }
        let timeout = rustix::event::Timespec {
            tv_sec: timeout.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: timeout.subsec_nanos().into(),
        };
        let borrowed: Vec<BorrowedFd<'_>> = self
            .devices
            .iter()
            .map(|device| unsafe { BorrowedFd::borrow_raw(device.as_raw_fd()) })
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
        for index in lost.into_iter().rev() {
            tracing::warn!(
                path = %self.devices[index].path().display(),
                "keyboard device disconnected"
            );
            self.devices.remove(index);
        }
        Ok(())
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
                self.pending.push_back(InputEvent::Key(chord));
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
            Some(KeyAction::Commit(_)) => Some(InputEvent::Boundary),
            Some(KeyAction::Text(text)) => Some(InputEvent::Text(text)),
            Some(KeyAction::Ignore) | None => None,
            Some(KeyAction::Unsupported) => Some(InputEvent::Boundary),
        };
        if let Some(event) = translated {
            self.pending.push_back(event);
        }
    }

    /// Whether any key is physically held right now.
    pub fn keys_held(&self) -> bool {
        !self.pressed.is_empty()
    }

    /// Blocks until every physically held key has been released, or until
    /// `timeout` elapses.
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
                // A key genuinely held down (or a missed release) must not
                // stall expansion forever.
                break;
            }
            self.poll_once(remaining.min(POLL_TIMEOUT))
                .map_err(|error| InputSourceError {
                    source: SOURCE_NAME,
                    retryable: error.is_retryable(),
                    message: error.to_string(),
                })?;
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
    fn enter_key_becomes_boundary_event() {
        let mut state = test_state();
        // KEY_ENTER = 28.
        assert_eq!(press(&mut state, 28), Some(InputEvent::Boundary));
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
    fn auto_repeat_does_not_clear_the_held_key() {
        let mut state = test_state();
        let mut source = EvdevSource {
            devices: Vec::new(),
            state: std::mem::replace(&mut state, test_state()),
            pending: VecDeque::new(),
            pressed: HashSet::new(),
        };
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 1));
        // Value 2 is kernel auto-repeat. It must not be mistaken for a release.
        source.translate(evdev::InputEvent::new(evdev::EventType::KEY.0, 30, 2));
        assert!(source.keys_held());
    }
}
