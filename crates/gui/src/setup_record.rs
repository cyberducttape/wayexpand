//! What first-run setup actually established, kept across restarts.
//!
//! `gui-prefs.toml` records only that the guide was finished or skipped, and
//! the desktop-test confirmation lived only in memory. This separate file
//! (`gui-setup.toml`, owner-only) records the outcome itself: whether the
//! user confirmed an exact insertion in a real application, or skipped the
//! guide. A verification keeps the application, typing route, build,
//! desktop session and date, so the GUI can trust it after a restart while
//! the environment is the same, and say what changed when it is not.
//!
//! It is a separate file because `gui-prefs.toml` is parsed strictly: a new
//! field there would make an older GUI discard every preference.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SetupOutcome {
    /// The user confirmed an exact insertion in a real application.
    Verified,
    /// The user dismissed the guide; nothing was verified.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SetupRecord {
    pub outcome: SetupOutcome,
    /// The application type the user tested in ("Text editor", ...).
    #[serde(default)]
    pub application: Option<String>,
    /// The typing route tested: "ibus", or "<source> → <injection>".
    #[serde(default)]
    pub route: Option<String>,
    /// The WayExpand build that was tested.
    #[serde(default)]
    pub build: Option<String>,
    /// Desktop and session type, such as "KDE · wayland".
    #[serde(default)]
    pub desktop: Option<String>,
    #[serde(default)]
    pub verified_at_unix: Option<u64>,
}

/// Why a saved verification no longer covers this environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Staleness {
    RouteChanged { was: String },
    DesktopChanged { was: String },
}

impl SetupRecord {
    pub(crate) fn verified(application: &str, route: Option<String>) -> Self {
        Self {
            outcome: SetupOutcome::Verified,
            application: Some(application.to_owned()),
            route,
            build: Some(current_build()),
            desktop: Some(current_desktop()),
            verified_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs()),
        }
    }

    pub(crate) fn skipped() -> Self {
        Self {
            outcome: SetupOutcome::Skipped,
            application: None,
            route: None,
            build: None,
            desktop: None,
            verified_at_unix: None,
        }
    }

    /// Whether this verification still covers `route` on this desktop.
    /// A new WayExpand build alone does not require a new test; a different
    /// typing route or desktop session does. An unknown current route says
    /// nothing either way, so it is treated as not yet covered.
    pub(crate) fn staleness(&self, route: Option<&str>, desktop: &str) -> Option<Staleness> {
        if let Some(was) = self.desktop.as_deref().filter(|was| *was != desktop) {
            return Some(Staleness::DesktopChanged {
                was: was.to_owned(),
            });
        }
        match (self.route.as_deref(), route) {
            (Some(was), Some(now)) if was != now => Some(Staleness::RouteChanged {
                was: was.to_owned(),
            }),
            _ => None,
        }
    }

    /// A verification that still covers the current route and desktop.
    pub(crate) fn covers(&self, route: Option<&str>, desktop: &str) -> bool {
        self.outcome == SetupOutcome::Verified
            && route.is_some()
            && self.staleness(route, desktop).is_none()
    }
}

/// `unix` seconds as a UTC calendar date, `YYYY-MM-DD`.
pub(crate) fn date(unix: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let days = (unix / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub(crate) fn current_build() -> String {
    format!(
        "{} ({})",
        env!("WAYEXPAND_BUILD_VERSION"),
        env!("WAYEXPAND_COMMIT")
    )
}

pub(crate) fn current_desktop() -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    match (desktop.is_empty(), session.is_empty()) {
        (false, false) => format!("{desktop} · {session}"),
        (false, true) => desktop,
        (true, false) => session,
        (true, true) => "unknown".into(),
    }
}

fn path() -> PathBuf {
    crate::settings::gui_state_dir().join("gui-setup.toml")
}

/// The saved record, or `None` when there is none or it cannot be read.
/// A damaged record only costs a re-test; it never blocks the GUI.
pub(crate) fn load() -> Option<SetupRecord> {
    let contents = fs::read_to_string(path()).ok()?;
    toml_edit::de::from_str(&contents).ok()
}

pub(crate) fn save(record: &SetupRecord) -> Result<(), String> {
    let path = path();
    let parent = path
        .parent()
        .ok_or_else(|| "setup record path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create preferences directory: {error}"))?;
    let contents = toml_edit::ser::to_string_pretty(record)
        .map_err(|error| format!("could not serialize the setup record: {error}"))?;
    crate::settings::write_gui_prefs_atomic(&path, &contents, |parent| {
        fs::File::open(parent)?.sync_all()
    })
}

#[cfg(test)]
mod tests {
    use super::{SetupOutcome, SetupRecord, Staleness};

    fn verified(route: &str, desktop: &str) -> SetupRecord {
        SetupRecord {
            outcome: SetupOutcome::Verified,
            application: Some("Text editor".into()),
            route: Some(route.into()),
            build: Some("1.3.6 (abc)".into()),
            desktop: Some(desktop.into()),
            verified_at_unix: Some(1),
        }
    }

    #[test]
    fn a_verification_covers_only_the_route_and_desktop_it_tested() {
        let record = verified("ibus", "KDE · wayland");
        assert!(record.covers(Some("ibus"), "KDE · wayland"));
        assert!(!record.covers(None, "KDE · wayland"), "unknown route");
        assert_eq!(
            record.staleness(Some("evdev → libei"), "KDE · wayland"),
            Some(Staleness::RouteChanged { was: "ibus".into() })
        );
        assert_eq!(
            record.staleness(Some("ibus"), "GNOME · wayland"),
            Some(Staleness::DesktopChanged {
                was: "KDE · wayland".into()
            })
        );
    }

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(super::date(0), "1970-01-01");
        assert_eq!(super::date(951_782_400), "2000-02-29");
        assert_eq!(super::date(1_794_614_400), "2026-11-14");
    }

    #[test]
    fn skipping_never_counts_as_verified() {
        assert!(!SetupRecord::skipped().covers(Some("ibus"), "KDE · wayland"));
    }

    #[test]
    fn the_record_round_trips_and_tolerates_missing_fields() {
        let record = verified("ibus", "KDE · wayland");
        let text = toml_edit::ser::to_string_pretty(&record).unwrap();
        assert_eq!(
            toml_edit::de::from_str::<SetupRecord>(&text).unwrap(),
            record
        );
        let minimal: SetupRecord = toml_edit::de::from_str("outcome = \"skipped\"\n").unwrap();
        assert_eq!(minimal, SetupRecord::skipped());
    }
}
