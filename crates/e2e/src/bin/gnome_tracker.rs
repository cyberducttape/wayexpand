//! `wx-e2e-gnome-tracker --expect APP_ID | --expect-unavailable`
//!
//! For `scripts/test-gnome-e2e.sh`: connects WayExpand's GNOME window
//! tracker to a private, headless GNOME Shell running the WayExpand
//! extension. `--expect` waits until the focused window reports APP_ID (the
//! initial snapshot from Subscribe counts) and prints its identity;
//! `--expect-unavailable` checks that the tracker refuses to connect, for
//! example after the extension was disabled. Prints one
//! "pass|fail <detail>" line and exits non-zero on failure.

use std::time::{Duration, Instant};

use wayexpand_backend_gnome_window::GnomeWindowTracker;
use wayexpand_core::WindowTracker;

const CONNECT_DEADLINE: Duration = Duration::from_secs(20);
const FOCUS_DEADLINE: Duration = Duration::from_secs(15);

fn fail(detail: impl std::fmt::Display) -> ! {
    println!("fail {detail}");
    std::process::exit(1);
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next()) {
        (Some("--expect"), Some(app_id)) => expect(&app_id),
        (Some("--expect-unavailable"), None) => match GnomeWindowTracker::probe() {
            Ok(()) => fail("tracker still available"),
            Err(error) => println!("pass tracker unavailable: {error}"),
        },
        _ => fail("usage: wx-e2e-gnome-tracker --expect APP_ID | --expect-unavailable"),
    }
}

fn expect(app_id: &str) {
    let connect_deadline = Instant::now() + CONNECT_DEADLINE;
    let mut tracker = loop {
        match GnomeWindowTracker::probe().and_then(|()| GnomeWindowTracker::new()) {
            Ok(tracker) => break tracker,
            Err(error) if Instant::now() < connect_deadline => {
                eprintln!("waiting for the GNOME tracker: {error}");
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(error) => fail(format!("tracker did not connect: {error}")),
        }
    };
    let deadline = Instant::now() + FOCUS_DEADLINE;
    let mut last = None;
    while Instant::now() < deadline {
        match tracker.next_window_timeout(Duration::from_millis(500)) {
            Ok(Some(Some(window))) => {
                if window.app_id.as_deref() == Some(app_id) {
                    match window.instance_id.as_deref() {
                        Some(instance) if instance.starts_with("mutter:") => {
                            println!("pass focused {app_id} ({instance})");
                            return;
                        }
                        other => fail(format!("{app_id} has no Mutter window ID: {other:?}")),
                    }
                }
                last = Some(window);
            }
            Ok(Some(None)) => last = None,
            Ok(None) => {}
            Err(error) => fail(format!("tracker failed: {error}")),
        }
    }
    fail(format!(
        "focused window never became {app_id}; last report: {last:?}"
    ));
}
