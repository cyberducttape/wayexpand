//! What first-run setup actually established, kept across restarts.
//!
//! `gui-prefs.toml` records only that the guide was finished or skipped, and
//! the desktop-test confirmation lived only in memory. This separate file
//! (`gui-setup.toml`, owner-only) records the outcome and evidence source:
//! whether a controlled client observed exact output, the user confirmed it
//! visually, or the guide was skipped. A verification keeps the application,
//! typing route, configuration fingerprint, build, desktop session and date,
//! so the GUI can trust it after
//! a restart while the environment and tested snippet library are unchanged.
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
    /// Exact insertion was observed or confirmed in a real application.
    Verified,
    /// The user dismissed the guide; nothing was verified.
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VerificationMethod {
    /// A controlled client read the exact resulting text without retaining it.
    MachineObserved,
    /// The user visually confirmed the exact result in an application.
    UserConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SetupRecord {
    pub outcome: SetupOutcome,
    /// Old records predate evidence provenance and represent user confirmation.
    #[serde(default)]
    pub verification_method: Option<VerificationMethod>,
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
    /// The run of the typing route that was tested: the IBus engine
    /// instance, or the daemon's input and output connection times. A
    /// restarted engine or daemon, even under the same route name, has not
    /// been verified.
    #[serde(default)]
    pub route_instance: Option<String>,
    /// Content-only digest of the configuration that was tested. Changing
    /// snippets or their filters requires a new real-application check.
    #[serde(default)]
    pub config_revision: Option<String>,
    #[serde(default)]
    pub verified_at_unix: Option<u64>,
    /// Local application checks, retained across later checks. Older setup
    /// records have no list; their single verified application is synthesized
    /// when the report is read.
    #[serde(default)]
    pub application_checks: Vec<LocalApplicationCheck>,
}

/// A user-confirmed or controlled-client observation for one named app.
/// This is local compatibility evidence, not official certification.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct LocalApplicationCheck {
    pub application: String,
    pub result: LocalTestResult,
    pub verification_method: VerificationMethod,
    pub route: Option<String>,
    pub route_instance: Option<String>,
    pub config_revision: Option<String>,
    pub build: String,
    pub desktop: String,
    pub session: Option<String>,
    pub checked_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LocalTestResult {
    Passed,
    NotConfirmed,
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
    /// The input engine or daemon restarted or reconnected since the test.
    RouteRestarted,
    BuildChanged {
        was: String,
    },
    ConfigurationChanged,
}

/// What the current session is running, to compare a verification against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scope<'a> {
    pub route: Option<&'a str>,
    pub route_instance: Option<&'a str>,
    pub config_revision: Option<&'a str>,
    pub desktop: &'a str,
    pub session: Option<&'a str>,
    pub build: &'a str,
}

impl SetupRecord {
    pub(crate) fn verified(application: &str, scope: &Scope<'_>) -> Self {
        Self::verified_with_method(application, scope, VerificationMethod::UserConfirmed)
    }

    pub(crate) fn machine_verified(application: &str, scope: &Scope<'_>) -> Self {
        Self::verified_with_method(application, scope, VerificationMethod::MachineObserved)
    }

    fn verified_with_method(
        application: &str,
        scope: &Scope<'_>,
        method: VerificationMethod,
    ) -> Self {
        let mut record = Self {
            outcome: SetupOutcome::Verified,
            verification_method: Some(method),
            application: Some(application.to_owned()),
            route: scope.route.map(str::to_owned),
            route_instance: scope.route_instance.map(str::to_owned),
            config_revision: scope.config_revision.map(str::to_owned),
            build: Some(scope.build.to_owned()),
            desktop: Some(scope.desktop.to_owned()),
            session: scope.session.map(str::to_owned),
            verified_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs()),
            application_checks: Vec::new(),
        };
        record.record_current_application_check();
        record
    }

    pub(crate) fn skipped() -> Self {
        Self {
            outcome: SetupOutcome::Skipped,
            verification_method: None,
            application: None,
            route: None,
            build: None,
            desktop: None,
            session: None,
            route_instance: None,
            config_revision: None,
            verified_at_unix: None,
            application_checks: Vec::new(),
        }
    }

    /// Carry forward local observations and replace the entry for this app
    /// with the current scope. Keep the report bounded for long-lived installs.
    pub(crate) fn inherit_application_checks(&mut self, previous: Option<&Self>) {
        let Some(previous) = previous else { return };
        self.application_checks = previous.application_checks.clone();
        if self.application_checks.is_empty() && previous.outcome == SetupOutcome::Verified {
            if let Some(application) = previous.application.as_deref() {
                self.application_checks.push(LocalApplicationCheck {
                    application: application.to_owned(),
                    result: LocalTestResult::Passed,
                    // Historical verified records predate evidence provenance
                    // and have always meant that the user confirmed the result.
                    verification_method: previous
                        .verification_method
                        .unwrap_or(VerificationMethod::UserConfirmed),
                    route: previous.route.clone(),
                    route_instance: previous.route_instance.clone(),
                    config_revision: previous.config_revision.clone(),
                    build: previous.build.clone().unwrap_or_default(),
                    desktop: previous.desktop.clone().unwrap_or_default(),
                    session: previous.session.clone(),
                    checked_at_unix: previous.verified_at_unix,
                });
            }
        }
        self.record_current_application_check();
    }

    fn record_current_application_check(&mut self) {
        if self.outcome != SetupOutcome::Verified {
            return;
        }
        let (Some(application), Some(method), Some(build), Some(desktop)) = (
            self.application.clone(),
            self.verification_method,
            self.build.clone(),
            self.desktop.clone(),
        ) else {
            return;
        };
        let check = LocalApplicationCheck {
            application,
            result: LocalTestResult::Passed,
            verification_method: method,
            route: self.route.clone(),
            route_instance: self.route_instance.clone(),
            config_revision: self.config_revision.clone(),
            build,
            desktop,
            session: self.session.clone(),
            checked_at_unix: self.verified_at_unix,
        };
        self.application_checks
            .retain(|old| !old.application.eq_ignore_ascii_case(&check.application));
        self.application_checks.push(check);
        if self.application_checks.len() > 64 {
            self.application_checks.remove(0);
        }
    }

    pub(crate) fn all_application_checks(&self) -> Vec<LocalApplicationCheck> {
        if !self.application_checks.is_empty() {
            return self.application_checks.clone();
        }
        if self.outcome == SetupOutcome::Verified {
            if let (Some(application), Some(build), Some(desktop)) = (
                self.application.clone(),
                self.build.clone(),
                self.desktop.clone(),
            ) {
                return vec![LocalApplicationCheck {
                    application,
                    result: LocalTestResult::Passed,
                    verification_method: self
                        .verification_method
                        .unwrap_or(VerificationMethod::UserConfirmed),
                    route: self.route.clone(),
                    route_instance: self.route_instance.clone(),
                    config_revision: self.config_revision.clone(),
                    build,
                    desktop,
                    session: self.session.clone(),
                    checked_at_unix: self.verified_at_unix,
                }];
            }
        }
        Vec::new()
    }

    pub(crate) fn record_application_observation(
        &mut self,
        application: &str,
        method: VerificationMethod,
        scope: &Scope<'_>,
        result: LocalTestResult,
    ) {
        let check = LocalApplicationCheck {
            application: application.to_owned(),
            result,
            verification_method: method,
            route: scope.route.map(str::to_owned),
            route_instance: scope.route_instance.map(str::to_owned),
            config_revision: scope.config_revision.map(str::to_owned),
            build: scope.build.to_owned(),
            desktop: scope.desktop.to_owned(),
            session: scope.session.map(str::to_owned),
            checked_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs()),
        };
        self.application_checks
            .retain(|old| !old.application.eq_ignore_ascii_case(application));
        self.application_checks.push(check);
        if self.application_checks.len() > 64 {
            self.application_checks.remove(0);
        }
    }

    pub(crate) fn application_check_is_current(
        check: &LocalApplicationCheck,
        scope: &Scope<'_>,
    ) -> bool {
        let record = Self {
            outcome: SetupOutcome::Verified,
            verification_method: Some(check.verification_method),
            application: Some(check.application.clone()),
            route: check.route.clone(),
            route_instance: check.route_instance.clone(),
            config_revision: check.config_revision.clone(),
            build: Some(check.build.clone()),
            desktop: Some(check.desktop.clone()),
            session: check.session.clone(),
            verified_at_unix: check.checked_at_unix,
            application_checks: Vec::new(),
        };
        record.staleness(scope).is_none()
    }

    /// Why this verification does not cover `scope`, or `None` when it does.
    /// A verification lasts for one compositor session, one configuration,
    /// one WayExpand build and one typing route on one desktop; anything missing from either
    /// side means a new test, never a match.
    pub(crate) fn staleness(&self, scope: &Scope<'_>) -> Option<Staleness> {
        let (
            Some(route),
            Some(instance),
            Some(config_revision),
            Some(desktop),
            Some(session),
            Some(build),
        ) = (
            self.route.as_deref(),
            self.route_instance.as_deref(),
            self.config_revision.as_deref(),
            self.desktop.as_deref(),
            self.session.as_deref(),
            self.build.as_deref(),
        )
        else {
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
        if scope.config_revision != Some(config_revision) {
            return Some(Staleness::ConfigurationChanged);
        }
        match scope.route {
            Some(now) if now != route => {
                return Some(Staleness::RouteChanged {
                    was: route.to_owned(),
                })
            }
            Some(_) => {}
            // An unknown current route says nothing; it is not a match.
            None => return Some(Staleness::Incomplete),
        }
        (scope.route_instance != Some(instance)).then_some(Staleness::RouteRestarted)
    }

    /// An insertion result that covers what is running now.
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
    use super::{LocalTestResult, Scope, SetupOutcome, SetupRecord, Staleness, VerificationMethod};

    fn verified(route: &str, desktop: &str) -> SetupRecord {
        SetupRecord {
            outcome: SetupOutcome::Verified,
            verification_method: Some(super::VerificationMethod::UserConfirmed),
            application: Some("Text editor".into()),
            route: Some(route.into()),
            build: Some("1.3.6 (abc)".into()),
            desktop: Some(desktop.into()),
            session: Some("boot:1".into()),
            route_instance: Some("ibus:42:100".into()),
            config_revision: Some("sha256:test-config".into()),
            verified_at_unix: Some(1),
            application_checks: Vec::new(),
        }
    }

    fn scope<'a>(route: Option<&'a str>, desktop: &'a str, session: Option<&'a str>) -> Scope<'a> {
        Scope {
            route,
            route_instance: Some("ibus:42:100"),
            config_revision: Some("sha256:test-config"),
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
    fn a_restarted_engine_or_daemon_needs_a_new_test() {
        let record = verified("ibus", "KDE · wayland");
        let restarted = Scope {
            route_instance: Some("ibus:43:200"),
            ..scope(Some("ibus"), "KDE · wayland", Some("boot:1"))
        };
        assert_eq!(
            record.staleness(&restarted),
            Some(Staleness::RouteRestarted)
        );
        let unknown = Scope {
            route_instance: None,
            ..scope(Some("ibus"), "KDE · wayland", Some("boot:1"))
        };
        assert_eq!(record.staleness(&unknown), Some(Staleness::RouteRestarted));
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
            SetupRecord {
                route_instance: None,
                ..complete.clone()
            },
            SetupRecord {
                config_revision: None,
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
        let legacy: SetupRecord = toml_edit::de::from_str(
            "outcome = \"verified\"\napplication = \"Kate\"\nroute = \"ibus\"\nbuild = \"1.3.6 (abc)\"\ndesktop = \"KDE · wayland\"\nsession = \"boot:1\"\nroute_instance = \"ibus:42:100\"\nconfig_revision = \"sha256:test-config\"\nverified_at_unix = 1\n",
        )
        .unwrap();
        assert_eq!(legacy.all_application_checks().len(), 1);
        assert_eq!(legacy.all_application_checks()[0].application, "Kate");
        assert_eq!(
            legacy.all_application_checks()[0].verification_method,
            VerificationMethod::UserConfirmed
        );
    }

    #[test]
    fn application_checks_survive_updates_and_are_scoped_to_the_current_run() {
        let current = scope(Some("ibus"), "KDE · wayland", Some("boot:1"));
        let mut previous = verified("ibus", "KDE · wayland");
        previous.application = Some("Kate".into());
        let mut latest = SetupRecord::verified("Firefox", &current);
        latest.inherit_application_checks(Some(&previous));
        let checks = latest.all_application_checks();
        assert_eq!(checks.len(), 2);
        assert!(SetupRecord::application_check_is_current(
            &checks[0], &current
        ));

        latest.record_application_observation(
            "Firefox",
            VerificationMethod::UserConfirmed,
            &current,
            LocalTestResult::NotConfirmed,
        );
        let firefox = latest
            .all_application_checks()
            .into_iter()
            .find(|check| check.application == "Firefox")
            .unwrap();
        assert_eq!(firefox.result, LocalTestResult::NotConfirmed);
        assert!(SetupRecord::application_check_is_current(
            &firefox, &current
        ));

        let other_session = scope(Some("ibus"), "KDE · wayland", Some("boot:2"));
        assert!(!SetupRecord::application_check_is_current(
            &firefox,
            &other_session
        ));

        let serialized = toml_edit::ser::to_string_pretty(&latest).unwrap();
        let restored: SetupRecord = toml_edit::de::from_str(&serialized).unwrap();
        assert_eq!(restored.all_application_checks().len(), 2);
        assert_eq!(
            restored
                .all_application_checks()
                .iter()
                .find(|check| check.application == "Firefox")
                .unwrap()
                .result,
            LocalTestResult::NotConfirmed
        );
    }
}
