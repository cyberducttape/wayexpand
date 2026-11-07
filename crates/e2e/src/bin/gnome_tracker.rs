//! `wx-e2e-gnome-tracker --expect-transition APP_ID | --expect-transition-empty | --expect-unavailable | --active-source | --active-layout | --activate-source KIND ID`
//!
//! For `scripts/test-gnome-e2e.sh`: connects WayExpand's GNOME window
//! tracker to a private, headless GNOME Shell running the WayExpand
//! extension. The transition checks subscribe before the test controller
//! changes focus, so their success depends on a GNOME Shell update event and
//! not only the initial snapshot. `--expect-unavailable` checks that the
//! tracker refuses to connect, for example after the extension was disabled. Prints one
//! "pass|fail <detail>" line and exits non-zero on failure.

use std::{
    io::Write,
    time::{Duration, Instant},
};

use wayexpand_backend_gnome_window::{
    activate_input_source, active_input_source, active_keyboard_layout, GnomeWindowTracker,
};
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
        (Some("--expect-transition"), Some(app_id)) => expect_transition(&app_id),
        (Some("--expect-transition-empty"), None) => expect_transition_empty(),
        (Some("--active-source"), None) => match active_input_source() {
            Ok((kind, id)) => println!("active-source {kind} {id}"),
            Err(error) => fail(format!("could not read active input source: {error}")),
        },
        (Some("--active-layout"), None) => match active_keyboard_layout() {
            Ok((kind, id, xkb_id, model, options)) => println!(
                "active-layout {kind} {id} {xkb_id} {model} {}",
                options.join(",")
            ),
            Err(error) => fail(format!("could not read active keyboard layout: {error}")),
        },
        (Some("--activate-source"), Some(kind)) => {
            let Some(id) = arguments.next() else {
                fail("--activate-source requires KIND ID");
            };
            if arguments.next().is_some() {
                fail("--activate-source accepts only KIND ID");
            }
            match activate_input_source(&kind, &id) {
                Ok((active_kind, active_id)) => {
                    println!("activated-source {active_kind} {active_id}")
                }
                Err(error) => fail(format!("could not activate input source: {error}")),
            }
        }
        (Some("--expect-unavailable"), None) => match GnomeWindowTracker::probe() {
            Ok(()) => fail("tracker still available"),
            Err(error) => println!("pass tracker unavailable: {error}"),
        },
        _ => fail(
            "usage: wx-e2e-gnome-tracker --expect-transition APP_ID | --expect-transition-empty | --expect-unavailable | --active-source | --active-layout | --activate-source KIND ID",
        ),
    }
}

fn connect_tracker() -> GnomeWindowTracker {
    let connect_deadline = Instant::now() + CONNECT_DEADLINE;
    loop {
        match GnomeWindowTracker::probe().and_then(|()| GnomeWindowTracker::new()) {
            Ok(tracker) => return tracker,
            Err(error) if Instant::now() < connect_deadline => {
                eprintln!("waiting for the GNOME tracker: {error}");
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(error) => fail(format!("tracker did not connect: {error}")),
        }
    }
}

fn initial_snapshot(tracker: &mut GnomeWindowTracker) -> Option<wayexpand_core::WindowContext> {
    match tracker.next_window_timeout(Duration::from_secs(5)) {
        Ok(Some(window)) => window,
        Ok(None) => fail("tracker did not deliver the initial focus snapshot"),
        Err(error) => fail(format!(
            "tracker failed before the initial focus snapshot: {error}"
        )),
    }
}

fn expect_transition(app_id: &str) {
    let mut tracker = connect_tracker();
    let initial = initial_snapshot(&mut tracker);
    if initial.as_ref().and_then(|window| window.app_id.as_deref()) == Some(app_id) {
        fail(format!(
            "initial focus already reports {app_id}; an actual focus transition is required"
        ));
    }
    println!("ready waiting for focused app {app_id}");
    let _ = std::io::stdout().flush();
    let deadline = Instant::now() + FOCUS_DEADLINE;
    let mut last = None;
    while Instant::now() < deadline {
        match tracker.next_window_timeout(Duration::from_millis(500)) {
            Ok(Some(Some(window))) => {
                if window.app_id.as_deref() == Some(app_id) {
                    match window.instance_id.as_deref() {
                        Some(instance) if instance.starts_with("mutter:") => {
                            println!("pass focus transition to {app_id} ({instance})");
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
        "GNOME did not report a focus transition to {app_id}; last report: {last:?}"
    ));
}

fn expect_transition_empty() {
    let mut tracker = connect_tracker();
    let initial = initial_snapshot(&mut tracker);
    if initial.is_none() {
        fail("initial focus snapshot is already empty; a focus-loss transition is required");
    }
    println!("ready waiting for focus loss");
    let _ = std::io::stdout().flush();
    let deadline = Instant::now() + FOCUS_DEADLINE;
    let mut last = initial;
    while Instant::now() < deadline {
        match tracker.next_window_timeout(Duration::from_millis(250)) {
            Ok(Some(None)) => {
                println!("pass focus-loss transition");
                return;
            }
            Ok(Some(Some(window))) => last = Some(window),
            Ok(None) => {}
            Err(error) => fail(format!(
                "tracker failed while waiting for focus clear: {error}"
            )),
        }
    }
    fail(format!(
        "focus did not clear after closing the test window; last report: {last:?}"
    ));
}
