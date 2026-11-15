//! Restarting IBus the way the desktop that owns it expects.
//!
//! On KDE Plasma (Wayland), KWin launches IBus as its input method
//! (`ibus-ui-gtk3 --enable-wayland-im --exec-daemon`). Neither IBus command
//! restarts it there: a plain `ibus restart` restarts the daemon directly and
//! waits for one KWin never relaunches, and `ibus restart --type=kde-wayland`
//! rewrites kwinrc and waits, with no timeout, for a KWin signal that never
//! comes when the setting did not change. KWin's own switch for its input
//! method, the writable `enabled` property of `org.kde.kwin.VirtualKeyboard`,
//! stops IBus and starts it again.

use std::thread;
use std::time::{Duration, Instant};

use crate::chain::{started_by, IbusDaemonProcess};

const KWIN_SERVICE: &str = "org.kde.KWin";
const KWIN_PATH: &str = "/VirtualKeyboard";
const KWIN_INTERFACE: &str = "org.kde.kwin.VirtualKeyboard";

/// How IBus is restarted on this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbusRestartMethod {
    /// KWin launched IBus; toggle KWin's input method.
    KwinInputMethod,
    /// Run `ibus restart`.
    IbusCommand,
    /// No ibus-daemon is running for this user.
    NotRunning,
}

/// Pick the restart method for the running ibus-daemon.
pub fn restart_method() -> IbusRestartMethod {
    match IbusDaemonProcess::find() {
        None => IbusRestartMethod::NotRunning,
        Some(daemon) if started_by(daemon.pid, "kwin_wayland") => {
            IbusRestartMethod::KwinInputMethod
        }
        Some(_) => IbusRestartMethod::IbusCommand,
    }
}

/// Restart a KWin-managed IBus by switching KWin's input method off and on,
/// and wait until a new ibus-daemon runs. Returns its PID.
///
/// The input method is switched back on whatever happens in between, so a
/// failure never leaves the desktop without its input method.
pub fn restart_through_kwin(timeout: Duration) -> Result<u32, String> {
    let previous = IbusDaemonProcess::find().map(|daemon| daemon.pid);
    let connection = zbus::blocking::Connection::session()
        .map_err(|error| format!("cannot reach the session bus: {error}"))?;
    let proxy = zbus::blocking::Proxy::new(&connection, KWIN_SERVICE, KWIN_PATH, KWIN_INTERFACE)
        .map_err(|error| format!("cannot reach KWin's input method: {error}"))?;
    let set_enabled = |enabled: bool| {
        proxy
            .set_property("enabled", enabled)
            .map_err(|error| format!("KWin refused to switch its input method: {error}"))
    };

    set_enabled(false)?;
    // KWin stops the input method asynchronously; wait briefly for the old
    // daemon to go so the new one is not mistaken for it.
    let stopped = wait_until(Duration::from_secs(5), || {
        IbusDaemonProcess::find().map(|daemon| daemon.pid) != previous || previous.is_none()
    });
    set_enabled(true)?;
    if !stopped {
        return Err("KWin did not stop IBus; its input method was switched back on".into());
    }
    let mut restarted = None;
    wait_until(timeout, || {
        restarted = IbusDaemonProcess::find()
            .map(|daemon| daemon.pid)
            .filter(|pid| Some(*pid) != previous);
        restarted.is_some()
    });
    restarted.ok_or_else(|| {
        format!(
            "KWin switched its input method back on, but no new ibus-daemon started within {} s; \
             check System Settings > Keyboard > Virtual Keyboard",
            timeout.as_secs()
        )
    })
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_stops_at_the_deadline_or_when_done() {
        let started = Instant::now();
        assert!(!wait_until(Duration::from_millis(250), || false));
        assert!(started.elapsed() >= Duration::from_millis(250));
        let mut calls = 0;
        assert!(wait_until(Duration::from_secs(5), || {
            calls += 1;
            calls == 3
        }));
        assert_eq!(calls, 3);
    }
}
