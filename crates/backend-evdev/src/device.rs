//! Thin adapter over `/dev/input/eventN` device discovery and reads. Kept
//! separate from `lib.rs` so the event-translation state machine in `lib.rs`
//! stays unit-testable without a real device.

use std::path::{Path, PathBuf};

use evdev::{Device, KeyCode};

pub struct KeyboardDevice {
    path: PathBuf,
    device: Device,
}

impl KeyboardDevice {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.device)
    }

    pub fn fetch_events(&mut self) -> std::io::Result<Vec<evdev::InputEvent>> {
        Ok(self.device.fetch_events()?.collect())
    }
}

/// A device counts as a keyboard only if it exposes the alphabetic key
/// range and Space. This excludes mice with a handful of extra buttons,
/// power/lid switches, and other `EV_KEY`-capable but non-keyboard devices
/// that would otherwise be swept up by a bare "supports EV_KEY" check.
fn is_keyboard(device: &Device) -> bool {
    let Some(keys) = device.supported_keys() else {
        return false;
    };
    keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_Z) && keys.contains(KeyCode::KEY_SPACE)
}

/// Discovery outcome, distinguishing "no keyboard hardware present" from
/// "keyboards exist but none were readable" so callers (notably `doctor`)
/// can give an actionable permission message instead of a generic failure.
pub struct Discovery {
    pub keyboards: Vec<KeyboardDevice>,
    pub permission_denied_paths: Vec<PathBuf>,
}

pub fn discover_keyboards() -> Discovery {
    let mut keyboards = Vec::new();
    let mut permission_denied_paths = Vec::new();
    for (path, device) in evdev::enumerate() {
        if !is_keyboard(&device) {
            continue;
        }
        keyboards.push(KeyboardDevice { path, device });
    }
    // evdev::enumerate() silently skips paths it could not open (including
    // ones denied by permissions), so re-scan the directory ourselves to
    // tell "no keyboard hardware" apart from "a keyboard exists but this
    // user cannot read it".
    if keyboards.is_empty() {
        if let Ok(entries) = std::fs::read_dir("/dev/input") {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !name.starts_with("event") {
                    continue;
                }
                if std::fs::File::open(&path).is_err() {
                    permission_denied_paths.push(path);
                }
            }
        }
    }
    Discovery {
        keyboards,
        permission_denied_paths,
    }
}
