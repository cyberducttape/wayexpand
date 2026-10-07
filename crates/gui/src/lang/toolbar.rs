use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn title(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet library",
            Language::German => "Snippet-Bibliothek",
        }
    }

    pub fn snippets_count(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("{} snippets", count),
            Language::German => format!("{} Snippets", count),
        }
    }

    pub fn hotkeys_count(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("{} hotkeys", count),
            Language::German => format!("{} Hotkeys", count),
        }
    }

    pub fn unsaved_changes(&self) -> &'static str {
        match self.lang {
            Language::English => "● Unsaved changes",
            Language::German => "● Ungespeicherte Änderungen",
        }
    }

    pub fn settings(&self) -> &'static str {
        match self.lang {
            Language::English => "Settings",
            Language::German => "Einstellungen",
        }
    }

    pub fn more_actions(&self) -> &'static str {
        match self.lang {
            Language::English => "More actions",
            Language::German => "Weitere Aktionen",
        }
    }

    pub fn sync_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Sync library",
            Language::German => "Bibliothek synchronisieren",
        }
    }

    pub fn sync_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Synchronizing the library with Git…",
            Language::German => "Bibliothek wird mit Git synchronisiert…",
        }
    }

    pub fn sync_save_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Save or discard the open edit before syncing",
            Language::German => {
                "Vor dem Synchronisieren die offene Änderung speichern oder verwerfen"
            }
        }
    }

    pub fn import_espanso(&self) -> &'static str {
        match self.lang {
            Language::English => "Import Espanso",
            Language::German => "Espanso importieren",
        }
    }

    pub fn diagnostics(&self) -> &'static str {
        match self.lang {
            Language::English => "Diagnostics",
            Language::German => "Diagnose",
        }
    }

    pub fn pause(&self) -> &'static str {
        match self.lang {
            Language::English => "Pause",
            Language::German => "Pause",
        }
    }

    pub fn resume(&self) -> &'static str {
        match self.lang {
            Language::English => "Resume",
            Language::German => "Fortsetzen",
        }
    }

    pub fn running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "● Running",
            Language::German => "● Aktiv",
        }
    }

    pub fn daemon_running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Running",
            Language::German => "Daemon: Aktiv",
        }
    }

    pub fn daemon_unreachable_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Unreachable",
            Language::German => "Daemon: Nicht erreichbar",
        }
    }

    pub fn daemon_not_enabled_status(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand: Not enabled",
            Language::German => "WayExpand: Nicht aktiviert",
        }
    }

    pub fn daemon_unknown_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Checking…",
            Language::German => "Daemon: Wird geprüft…",
        }
    }

    pub fn route_trust_status(
        &self,
        route: &str,
        maturity: &str,
        sensitive_fields: bool,
        atomic_replace: bool,
        certified: bool,
    ) -> String {
        let semantics = match (self.lang, sensitive_fields, atomic_replace) {
            (Language::English, true, true) => "sensitive-field aware · atomic replacement",
            (Language::English, true, false) => "sensitive-field aware · non-atomic replacement",
            (Language::English, false, true) => "global keyboard visibility · atomic replacement",
            (Language::English, false, false) => {
                "global keyboard visibility · non-atomic replacement"
            }
            (Language::German, true, true) => "passwortfeldbewusst · atomare Ersetzung",
            (Language::German, true, false) => "passwortfeldbewusst · nicht-atomare Ersetzung",
            (Language::German, false, true) => "globale Tastatursichtbarkeit · atomare Ersetzung",
            (Language::German, false, false) => {
                "globale Tastatursichtbarkeit · nicht-atomare Ersetzung"
            }
        };
        let evidence = match (self.lang, certified) {
            (Language::English, true) => "certified",
            (Language::English, false) => "certification pending",
            (Language::German, true) => "zertifiziert",
            (Language::German, false) => "Zertifizierung ausstehend",
        };
        format!("{route} · {maturity} · {semantics} · {evidence}")
    }

    pub fn route_connected_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Ready",
            Language::German => "Tastaturintegration: Bereit",
        }
    }

    /// Connected with full protection, but the route is not certified, so
    /// expansion has not been verified on this desktop.
    pub fn route_running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Running",
            Language::German => "Tastaturintegration: Aktiv",
        }
    }
    pub fn route_limited_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Limited protection",
            Language::German => "Tastaturintegration: Eingeschränkter Schutz",
        }
    }
    pub fn route_paused_status(&self) -> &'static str {
        "Typing integration: Paused"
    }
    pub fn route_reconnecting_status(&self) -> &'static str {
        "Typing integration: Reconnecting…"
    }
    pub fn route_starting_status(&self) -> &'static str {
        "Typing integration: Starting…"
    }

    pub fn route_permission_required_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Permission required",
            Language::German => "Tastaturintegration: Berechtigung erforderlich",
        }
    }

    pub fn route_portal_revoked_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Permission revoked",
            Language::German => "Tastaturintegration: Berechtigung widerrufen",
        }
    }

    pub fn route_unsupported_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Unsupported",
            Language::German => "Tastaturintegration: Nicht unterstützt",
        }
    }
    pub fn route_degraded_status(&self) -> &'static str {
        "Typing integration: Degraded"
    }
    pub fn route_failed_status(&self) -> &'static str {
        "Typing integration: Failed"
    }
    pub fn route_stopped_status(&self) -> &'static str {
        "Typing integration: Stopped"
    }
    pub fn route_unknown_status(&self) -> &'static str {
        "Typing integration: Unknown"
    }

    pub fn technical_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Technical details",
            Language::German => "Technische Details",
        }
    }

    pub fn backend_label(&self, kind: wayexpand_core::BackendKind) -> &'static str {
        match (self.lang, kind) {
            (
                Language::English,
                wayexpand_core::BackendKind::InputMethodV2 | wayexpand_core::BackendKind::Evdev,
            ) => "Keyboard capture",
            (
                Language::German,
                wayexpand_core::BackendKind::InputMethodV2 | wayexpand_core::BackendKind::Evdev,
            ) => "Tastatureingabe",
            (
                Language::English,
                wayexpand_core::BackendKind::Libei
                | wayexpand_core::BackendKind::WlrootsVirtualKeyboard
                | wayexpand_core::BackendKind::Uinput,
            ) => "Text injection",
            (
                Language::German,
                wayexpand_core::BackendKind::Libei
                | wayexpand_core::BackendKind::WlrootsVirtualKeyboard
                | wayexpand_core::BackendKind::Uinput,
            ) => "Texteingabe",
            (Language::English, wayexpand_core::BackendKind::WindowTracker) => {
                "Application detection"
            }
            (Language::German, wayexpand_core::BackendKind::WindowTracker) => "App-Erkennung",
            (Language::English, wayexpand_core::BackendKind::Clipboard) => "Clipboard",
            (Language::German, wayexpand_core::BackendKind::Clipboard) => "Zwischenablage",
        }
    }

    pub fn reload(&self) -> &'static str {
        match self.lang {
            Language::English => "Reload",
            Language::German => "Neu laden",
        }
    }

    pub fn search_placeholder(&self) -> &'static str {
        match self.lang {
            Language::English => "Search snippets…",
            Language::German => "Snippets durchsuchen…",
        }
    }

    pub fn search_fields(&self) -> &'static str {
        match self.lang {
            Language::English => "Search fields",
            Language::German => "Suchfelder",
        }
    }

    pub fn search_triggers(&self) -> &'static str {
        match self.lang {
            Language::English => "Triggers",
            Language::German => "Kürzel",
        }
    }

    pub fn search_descriptions(&self) -> &'static str {
        match self.lang {
            Language::English => "Names and descriptions",
            Language::German => "Namen und Beschreibungen",
        }
    }

    pub fn search_tags(&self) -> &'static str {
        match self.lang {
            Language::English => "Tags",
            Language::German => "Schlagwörter",
        }
    }

    pub fn search_replacements(&self) -> &'static str {
        match self.lang {
            Language::English => "Replacement content",
            Language::German => "Eingefügter Text",
        }
    }

    pub fn welcome_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Welcome to WayExpand",
            Language::German => "Willkommen bei WayExpand",
        }
    }

    pub fn welcome_intro(&self) -> &'static str {
        match self.lang {
            Language::English => "Text expansion for Wayland, in three steps.",
            Language::German => "Textexpansion für Wayland, in drei Schritten.",
        }
    }

    pub fn select_snippet_prompt(&self) -> &'static str {
        match self.lang {
            Language::English => "Select a snippet from your library to edit it.",
            Language::German => "Wählen Sie ein Snippet aus Ihrer Bibliothek zum Bearbeiten aus.",
        }
    }

    pub fn onboarding_desktop(&self, desktop: &str, wayland: bool) -> String {
        match (self.lang, wayland) {
            (Language::English, true) => format!("{desktop} · Wayland session detected"),
            (Language::English, false) => format!("{desktop} · Wayland session not detected"),
            (Language::German, true) => format!("{desktop} · Wayland-Sitzung erkannt"),
            (Language::German, false) => format!("{desktop} · Keine Wayland-Sitzung erkannt"),
        }
    }

    pub fn onboarding_probe_caveat(&self) -> &'static str {
        match self.lang {
            Language::English => "Backend probes show what may be available; they do not certify reliable input or output.",
            Language::German => "Backend-Prüfungen zeigen mögliche Verfügbarkeit, zertifizieren aber keine zuverlässige Ein- oder Ausgabe.",
        }
    }

    pub fn onboarding_app_caveat(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Application filters depend on desktop integration and may be unavailable."
            }
            Language::German => {
                "App-Filter benötigen Desktop-Integration und sind möglicherweise nicht verfügbar."
            }
        }
    }

    pub fn create_test_snippet(&self) -> &'static str {
        match self.lang {
            Language::English => "Create test snippet",
            Language::German => "Testsnippet erstellen",
        }
    }

    pub fn onboarding_try_text(&self) -> &'static str {
        match self.lang {
            Language::English => "Type :wayexpand-test in the Matcher preview box, then use the separate desktop setup checks before trying it in another app. Never test in a password field.",
            Language::German => "Tippen Sie :wayexpand-test in der Matcher-Vorschau ein und verwenden Sie danach die getrennten Desktop-Tests, bevor Sie es in einer anderen App probieren. Nie in einem Passwortfeld testen.",
        }
    }

    pub fn onboarding_sample_description(&self) -> &'static str {
        match self.lang {
            Language::English => "A test expansion for your first-run check",
            Language::German => "Eine Textersetzung zum Ausprobieren",
        }
    }

    pub fn onboarding_sample_category(&self) -> &'static str {
        match self.lang {
            Language::English => "Getting started",
            Language::German => "Erste Schritte",
        }
    }

    pub fn onboarding_certification_note(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Manual verification only · this desktop is not automatically certified."
            }
            Language::German => {
                "Nur manuell geprüft · dieser Desktop ist nicht automatisiert zertifiziert."
            }
        }
    }

    pub fn onboarding_detection(&self, available: bool) -> &'static str {
        match (self.lang, available) {
            (Language::English, true) => "Application detection probe available",
            (Language::English, false) => "Application detection unavailable on this desktop",
            (Language::German, true) => "Prüfung der App-Erkennung verfügbar",
            (Language::German, false) => "App-Erkennung auf diesem Desktop nicht verfügbar",
        }
    }

    pub fn onboarding_keyboard_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Keyboard capture",
            Language::German => "Tastatureingabe",
        }
    }

    pub fn onboarding_recommended_route(
        &self,
        recommendation: Option<RouteRecommendation>,
    ) -> (String, String) {
        let Some(route) = recommendation else {
            return match self.lang {
                Language::English => (
                    "No complete safe route detected yet".into(),
                    "Refresh diagnostics after starting the desktop session".into(),
                ),
                Language::German => (
                    "Noch kein vollständiger sicherer Weg erkannt".into(),
                    "Diagnose nach dem Start der Desktop-Sitzung aktualisieren".into(),
                ),
            };
        };
        let state = if matches!(
            (route.capture_state, route.injection_state),
            (BackendState::Available, BackendState::Available)
        ) {
            match self.lang {
                Language::English => "Detected on this machine",
                Language::German => "Auf diesem Rechner erkannt",
            }
        } else {
            match self.lang {
                Language::English => "Permission required before setup",
                Language::German => "Vor der Einrichtung ist eine Berechtigung nötig",
            }
        };
        let label = match self.lang {
            Language::English => "Safe integration".to_owned(),
            Language::German => "Sichere Integration".to_owned(),
        };
        let detail = match self.lang {
            Language::English => {
                let mut properties = vec![state.to_owned()];
                if route.sensitive_fields {
                    properties.push("best protection for unsupported or sensitive contexts".into());
                }
                if route.atomic_replace {
                    properties.push("atomic replacement".into());
                }
                if route.focus_tracking {
                    properties.push("application focus tracking".into());
                }
                properties.push(if route.certified {
                    "certified".into()
                } else {
                    "experimental until certified".into()
                });
                properties.join(" · ")
            }
            Language::German => {
                let mut properties = vec![state.to_owned()];
                if route.sensitive_fields {
                    properties
                        .push("bester Schutz für nicht unterstützte oder sensible Kontexte".into());
                }
                if route.atomic_replace {
                    properties.push("atomare Ersetzung".into());
                }
                if route.focus_tracking {
                    properties.push("Verfolgung des App-Fokus".into());
                }
                properties.push(if route.certified {
                    "zertifiziert".into()
                } else {
                    "experimentell bis zur Zertifizierung".into()
                });
                properties.join(" · ")
            }
        };
        (label, detail)
    }

    pub fn onboarding_recommended_route_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Recommended integration",
            Language::German => "Empfohlene Integration",
        }
    }

    pub fn onboarding_injection_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Text injection",
            Language::German => "Texteingabe",
        }
    }

    pub fn onboarding_capture_detail(&self, backend: &str) -> String {
        match self.lang {
            Language::English => format!("Capture: {backend}"),
            Language::German => format!("Eingabe: {backend}"),
        }
    }

    pub fn onboarding_injection_detail(&self, backend: &str) -> String {
        match self.lang {
            Language::English => format!("Output: {backend}"),
            Language::German => format!("Ausgabe: {backend}"),
        }
    }

    pub fn onboarding_backend_state(&self, state: wayexpand_core::BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English, wayexpand_core::BackendState::Available) => "Detected",
            (Language::German, wayexpand_core::BackendState::Available) => "Erkannt",
            (Language::English, wayexpand_core::BackendState::RequiresPermission) => {
                "Permission required"
            }
            (Language::German, wayexpand_core::BackendState::RequiresPermission) => {
                "Berechtigung erforderlich"
            }
            (Language::English, wayexpand_core::BackendState::Implemented) => {
                "Supported for testing · not verified"
            }
            (Language::German, wayexpand_core::BackendState::Implemented) => {
                "Zum Testen unterstützt · nicht geprüft"
            }
            (
                Language::English,
                wayexpand_core::BackendState::Unavailable
                | wayexpand_core::BackendState::NotImplemented,
            ) => "Unavailable",
            (
                Language::German,
                wayexpand_core::BackendState::Unavailable
                | wayexpand_core::BackendState::NotImplemented,
            ) => "Nicht verfügbar",
        }
    }

    pub fn onboarding_safety_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Try it",
            Language::German => "Ausprobieren",
        }
    }

    pub fn onboarding_evdev_setup(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable raw keyboard capture — advanced…",
            Language::German => "Rohe Tastaturerfassung aktivieren — erweitert…",
        }
    }

    pub fn evdev_setup_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable raw keyboard capture — advanced",
            Language::German => "Rohe Tastaturerfassung aktivieren — erweitert",
        }
    }

    pub fn evdev_setup_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Works in more applications, but can observe password-field typing because this mode has no sensitive-field signal. This is an explicit opt-in. Device permissions are a separate system-wide change and are never made by this GUI.",
            Language::German => "Funktioniert in mehr Anwendungen, kann aber Eingaben in Passwortfeldern beobachten, weil dieser Modus kein Signal für sensible Felder hat. Dies ist eine ausdrückliche Zustimmung. Geräteberechtigungen sind eine separate systemweite Änderung und werden von dieser GUI niemals vorgenommen.",
        }
    }

    pub fn evdev_setup_acknowledge(&self) -> &'static str {
        match self.lang {
            Language::English => "I understand the raw-input and password-field limitations",
            Language::German => "Ich verstehe die Einschränkungen bei Rohdaten und Passwortfeldern",
        }
    }

    pub fn evdev_setup_steps(&self) -> &'static str {
        match self.lang {
            Language::English => "If keyboard-device access is not already configured, first complete the administrator permission step in Getting Started. Then copy the command below into a terminal: it enables the user service, and the desktop may ask you to approve the input portal. Return here, refresh diagnostics, and test in a normal text field.",
            Language::German => "Falls der Zugriff auf Tastaturgeräte noch nicht eingerichtet ist, schließen Sie zuerst den Administrator-Schritt in der Installationsanleitung ab. Kopieren Sie danach den folgenden Befehl in ein Terminal: Er aktiviert den Benutzerdienst; der Desktop kann um Freigabe des Eingabeportals bitten. Kehren Sie zurück, aktualisieren Sie die Diagnose und testen Sie in einem normalen Textfeld.",
        }
    }
}
