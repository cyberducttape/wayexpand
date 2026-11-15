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
    /// The compositor session the test ran in (see `current_session`).
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub verified_at_unix: Option<u64>,
}

/// Why a saved verification no longer covers this environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Staleness {
    /// The record lacks what is needed to say what it covered.
    Incomplete,
    RouteChanged {
        was: String,
    },
    DesktopChanged {
        was: String,
    },
    /// A new login or compositor start since the test.
    SessionChanged,
    BuildChanged {
        was: String,
    },
}

/// What the current session is running, to compare a verification against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scope<'a> {
    pub route: Option<&'a str>,
    pub desktop: &'a str,
    pub session: Option<&'a str>,
    pub build: &'a str,
}

impl SetupRecord {
    pub(crate) fn verified(application: &str, route: Option<String>) -> Self {
        Self {
            outcome: SetupOutcome::Verified,
            application: Some(application.to_owned()),
            route,
            build: Some(current_build()),
            desktop: Some(current_desktop()),
            session: current_session(),
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
            session: None,
            verified_at_unix: None,
        }
    }

    /// Why this verification does not cover `scope`, or `None` when it does.
    /// A verification lasts for one compositor session, one WayExpand build
    /// and one typing route on one desktop; anything missing from either
    /// side means a new test, never a match.
    pub(crate) fn staleness(&self, scope: &Scope<'_>) -> Option<Staleness> {
        let (Some(route), Some(desktop), Some(session), Some(build)) = (
            self.route.as_deref(),
            self.desktop.as_deref(),
            self.session.as_deref(),
            self.build.as_deref(),
        ) else {
            return Some(Staleness::Incomplete);
        };
        if desktop != scope.desktop {
            return Some(Staleness::DesktopChanged {
                was: desktop.to_owned(),
            });
        }
        if scope.session != Some(session) {
            return Some(Staleness::SessionChanged);
        }
        if build != scope.build {
            return Some(Staleness::BuildChanged {
                was: build.to_owned(),
            });
        }
        match scope.route {
            Some(now) if now == route => None,
            Some(_) => Some(Staleness::RouteChanged {
                was: route.to_owned(),
            }),
            // An unknown current route says nothing; it is not a match.
            None => Some(Staleness::Incomplete),
        }
    }

    /// A user-confirmed insertion that covers what is running now.
    #[cfg(test)]
    pub(crate) fn covers(&self, scope: &Scope<'_>) -> bool {
        self.outcome == SetupOutcome::Verified && self.staleness(scope).is_none()
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

/// The compositor session: the boot plus the creation time of the Wayland
/// socket, which the compositor makes when it starts. A new login, a
/// compositor restart (such as after a desktop update) or a reboot changes
/// it. `None` outside a Wayland session.
pub(crate) fn current_session() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var_os("WAYLAND_DISPLAY")?;
    let socket = std::path::Path::new(&runtime).join(display);
    let created = fs::metadata(socket).ok()?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|id| id.trim().to_owned())
        .unwrap_or_default();
    Some(format!(
        "{boot}:{}.{}",
        created.ctime(),
        created.ctime_nsec()
    ))
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
    use super::{Scope, SetupOutcome, SetupRecord, Staleness};

    fn verified(route: &str, desktop: &str) -> SetupRecord {
        SetupRecord {
            outcome: SetupOutcome::Verified,
            application: Some("Text editor".into()),
            route: Some(route.into()),
            build: Some("1.3.6 (abc)".into()),
            desktop: Some(desktop.into()),
            session: Some("boot:1".into()),
            verified_at_unix: Some(1),
        }
    }

    fn scope<'a>(route: Option<&'a str>, desktop: &'a str, session: Option<&'a str>) -> Scope<'a> {
        Scope {
            route,
            desktop,
            session,
            build: "1.3.6 (abc)",
        }
    }

    #[test]
    fn a_verification_covers_one_session_build_route_and_desktop() {
        let record = verified("ibus", "KDE · wayland");
        assert!(record.covers(&scope(Some("ibus"), "KDE · wayland", Some("boot:1"))));
        assert!(
            !record.covers(&scope(None, "KDE · wayland", Some("boot:1"))),
            "unknown route"
        );
        assert_eq!(
            record.staleness(&scope(
                Some("evdev → libei"),
                "KDE · wayland",
                Some("boot:1")
            )),
            Some(Staleness::RouteChanged { was: "ibus".into() })
        );
        assert_eq!(
            record.staleness(&scope(Some("ibus"), "GNOME · wayland", Some("boot:1"))),
            Some(Staleness::DesktopChanged {
                was: "KDE · wayland".into()
            })
        );
        // Logging out and back in, with the same desktop name, is a new
        // session.
        assert_eq!(
            record.staleness(&scope(Some("ibus"), "KDE · wayland", Some("boot:2"))),
            Some(Staleness::SessionChanged)
        );
        assert_eq!(
            record.staleness(&Scope {
                build: "1.3.7 (def)",
                ..scope(Some("ibus"), "KDE · wayland", Some("boot:1"))
            }),
            Some(Staleness::BuildChanged {
                was: "1.3.6 (abc)".into()
            })
        );
    }

    #[test]
    fn a_record_missing_any_scope_field_never_counts() {
        let current = scope(Some("ibus"), "KDE · wayland", Some("boot:1"));
        let complete = verified("ibus", "KDE · wayland");
        for incomplete in [
            SetupRecord {
                route: None,
                ..complete.clone()
            },
            SetupRecord {
                desktop: None,
                ..complete.clone()
            },
            SetupRecord {
                session: None,
                ..complete.clone()
            },
            SetupRecord {
                build: None,
                ..complete.clone()
            },
        ] {
            assert_eq!(incomplete.staleness(&current), Some(Staleness::Incomplete));
            assert!(!incomplete.covers(&current));
        }
    }

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(super::date(0), "1970-01-01");
        assert_eq!(super::date(951_782_400), "2000-02-29");
        assert_eq!(super::date(1_794_614_400), "2026-11-14");
    }

    #[test]
    fn skipping_never_counts_as_verified() {
        assert!(!SetupRecord::skipped().covers(&scope(
            Some("ibus"),
            "KDE · wayland",
            Some("boot:1")
        )));
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
