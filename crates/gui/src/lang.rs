use wayexpand_core::{BackendState, FontScale};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    pub fn from_env() -> Self {
        std::env::var("LANG")
            .ok()
            .and_then(|lang| {
                if lang.starts_with("de") {
                    Some(Language::German)
                } else {
                    None
                }
            })
            .unwrap_or(Language::English)
    }

    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::English),
            "de" => Some(Language::German),
            _ => None,
        }
    }
}

pub struct Strings {
    lang: Language,
}

impl Strings {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }

    pub fn set_language(&mut self, lang: Language) {
        self.lang = lang;
    }

    // Toolbar
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

    pub fn disconnected_status(&self) -> &'static str {
        match self.lang {
            Language::English => "● Disconnected",
            Language::German => "● Getrennt",
        }
    }

    pub fn status_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "◌ Checking",
            Language::German => "◌ Wird geprüft",
        }
    }

    pub fn paused_status(&self) -> &'static str {
        match self.lang {
            Language::English => "● Paused",
            Language::German => "● Pausiert",
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

    pub fn daemon_unknown_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Checking…",
            Language::German => "Daemon: Wird geprüft…",
        }
    }

    pub fn route_connected_status(&self) -> &'static str {
        "Typing integration: Ready"
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
            Language::English => "Fields",
            Language::German => "Felder",
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
            Language::English => "Set up a test expansion, then verify it in an app you use every day.",
            Language::German => "Richten Sie eine Test-Ersetzung ein und prüfen Sie sie in einer täglich genutzten App.",
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
            Language::English => "Then type :wayexpand-test in another app. It expands to “WayExpand is working!” Use a normal text field, never a password field.",
            Language::German => "Geben Sie danach :wayexpand-test in einer anderen App ein. Es wird zu „WayExpand is working!“. Verwenden Sie ein normales Textfeld, kein Passwortfeld.",
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

    pub fn onboarding_support_title(&self) -> &'static str {
        match self.lang {
            Language::English => "2 · Input and application support",
            Language::German => "2 · Eingabe und App-Unterstützung",
        }
    }

    pub fn onboarding_desktop_step(&self) -> &'static str {
        match self.lang {
            Language::English => "1 · Desktop",
            Language::German => "1 · Desktop",
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
        keyboard: wayexpand_core::BackendState,
        injection: wayexpand_core::BackendState,
    ) -> (&'static str, &'static str) {
        let ready = matches!(keyboard, wayexpand_core::BackendState::Available)
            && matches!(injection, wayexpand_core::BackendState::Available);
        let permission = matches!(
            keyboard,
            wayexpand_core::BackendState::RequiresPermission
        ) || matches!(
            injection,
            wayexpand_core::BackendState::RequiresPermission
        );
        match (self.lang, ready, permission) {
            (Language::English, true, _) => (
                "Input Method v2 → libei via the desktop portal",
                "Detected on this machine",
            ),
            (Language::German, true, _) => (
                "Input Method v2 → libei über das Desktop-Portal",
                "Auf diesem Rechner erkannt",
            ),
            (Language::English, false, true) => (
                "Safest detected route requires permission",
                "Permission required before setup",
            ),
            (Language::German, false, true) => (
                "Sicherster erkannter Weg benötigt eine Berechtigung",
                "Vor der Einrichtung ist eine Berechtigung nötig",
            ),
            (Language::English, false, false) => (
                "No complete safe route detected yet",
                "Refresh diagnostics after starting the desktop session",
            ),
            (Language::German, false, false) => (
                "Noch kein vollständiger sicherer Weg erkannt",
                "Diagnose nach dem Start der Desktop-Sitzung aktualisieren",
            ),
        }
    }

    pub fn onboarding_recommended_route_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Recommended input route",
            Language::German => "Empfohlener Eingabeweg",
        }
    }

    pub fn onboarding_injection_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Text injection",
            Language::German => "Texteingabe",
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
            Language::English => "3 · Safe expansion check",
            Language::German => "3 · Sicherer Ersetzungstest",
        }
    }

    pub fn onboarding_evdev_setup(&self) -> &'static str {
        match self.lang {
            Language::English => "Set up broad application coverage…",
            Language::German => "Breite App-Unterstützung einrichten…",
        }
    }

    pub fn evdev_setup_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Maximum compatibility uses raw keyboard input",
            Language::German => "Maximale Kompatibilität verwendet rohe Tastatureingaben",
        }
    }

    pub fn evdev_setup_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand can observe typing in password fields in this mode and cannot detect sensitive fields. This is an explicit opt-in. Device permissions are a separate system-wide change and are never made by this GUI.",
            Language::German => "WayExpand kann in diesem Modus Eingaben in Passwortfeldern beobachten und sensible Felder nicht erkennen. Dies ist eine ausdrückliche Zustimmung. Geräteberechtigungen sind eine separate systemweite Änderung und werden von dieser GUI niemals vorgenommen.",
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

    // Dialogs
    pub fn diagnostics_title(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand diagnostics",
            Language::German => "WayExpand Diagnose",
        }
    }

    pub fn runtime_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Runtime health",
            Language::German => "Laufzeit-Zustand",
        }
    }

    pub fn compatibility_center(&self) -> &'static str {
        match self.lang {
            Language::English => "Compatibility Center",
            Language::German => "Kompatibilitätszentrum",
        }
    }

    pub fn run_compatibility_test(&self) -> &'static str {
        match self.lang {
            Language::English => "Run compatibility checks",
            Language::German => "Kompatibilitätsprüfung ausführen",
        }
    }

    pub fn active_route(&self) -> &'static str {
        match self.lang {
            Language::English => "Active route",
            Language::German => "Aktiver Weg",
        }
    }

    pub fn configuration_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration",
            Language::German => "Konfiguration",
        }
    }

    pub fn configuration_healthy(&self) -> &'static str {
        match self.lang {
            Language::English => "Healthy",
            Language::German => "In Ordnung",
        }
    }

    pub fn configuration_invalid(&self) -> &'static str {
        match self.lang {
            Language::English => "Invalid",
            Language::German => "Ungültig",
        }
    }

    pub fn safety(&self) -> &'static str {
        match self.lang {
            Language::English => "Safety",
            Language::German => "Sicherheit",
        }
    }

    pub fn capture_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English => "Keyboard capture",
            Language::German => "Tastaturerfassung",
        }
    }

    pub fn injection_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English => "Text output",
            Language::German => "Textausgabe",
        }
    }

    pub fn application_context(&self) -> &'static str {
        match self.lang {
            Language::English => "Application context",
            Language::German => "Anwendungskontext",
        }
    }

    pub fn capability_label(&self, key: &str) -> &'static str {
        match (self.lang, key) {
            (Language::English, "sensitive_focus") => "Sensitive-field awareness",
            (Language::German, "sensitive_focus") => "Erkennung sensibler Felder",
            (Language::English, "exclusive") => "Exclusive keyboard capture",
            (Language::German, "exclusive") => "Exklusive Tastaturerfassung",
            (Language::English, "reliable_key_state") => "Key press/release tracking",
            (Language::German, "reliable_key_state") => "Tastenanschlag/-loslassen verfolgen",
            (Language::English, "capture_passthrough") => "Unsupported-key forwarding",
            (Language::German, "capture_passthrough") => "Weitergabe nicht unterstützter Tasten",
            (Language::English, "composition") => "IME composition awareness",
            (Language::German, "composition") => "IME-Kompositionserkennung",
            (Language::English, "window_tracker") => "KWin window tracker",
            (Language::German, "window_tracker") => "KWin-Fensterverfolgung",
            (Language::English, "atomic_replace") => "Atomic text replacement",
            (Language::German, "atomic_replace") => "Atomarer Textersatz",
            (Language::English, "unicode") => "Layout-independent Unicode",
            (Language::German, "unicode") => "Tastaturlayout-unabhängiges Unicode",
            (Language::English, "cursor") => "Cursor repositioning",
            (Language::German, "cursor") => "Cursorpositionierung",
            (Language::English, "injection_passthrough") => "Key press/release forwarding",
            (Language::German, "injection_passthrough") => "Weitergabe von Tastenanschlägen",
            _ => "",
        }
    }

    pub fn capability_available(&self) -> &'static str {
        match self.lang {
            Language::English => "Available",
            Language::German => "Verfügbar",
        }
    }

    pub fn capability_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Not provided",
            Language::German => "Nicht verfügbar",
        }
    }

    pub fn capability_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "Unknown · update/restart daemon",
            Language::German => "Unbekannt · Dienst aktualisieren/neu starten",
        }
    }

    pub fn daemon(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon",
            Language::German => "Daemon",
        }
    }

    pub fn backends(&self) -> &'static str {
        match self.lang {
            Language::English => "Backends",
            Language::German => "Backends",
        }
    }

    pub fn refresh(&self) -> &'static str {
        match self.lang {
            Language::English => "Refresh",
            Language::German => "Aktualisieren",
        }
    }

    pub fn protocol_probes(&self) -> &'static str {
        match self.lang {
            Language::English => "Non-mutating protocol probes",
            Language::German => "Nicht-mutierende Protokoll-Tests",
        }
    }

    pub fn import_dialog_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Import Espanso library",
            Language::German => "Espanso-Bibliothek importieren",
        }
    }

    pub fn source_yaml(&self) -> &'static str {
        match self.lang {
            Language::English => "Source YAML file",
            Language::German => "YAML-Quelldatei",
        }
    }

    pub fn import_preview_info(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview first, then merge while preserving current snippets or explicitly replace the library.",
            Language::German => "Zuerst prüfen, dann aktuelle Snippets beim Zusammenführen bewahren oder die Bibliothek ausdrücklich ersetzen.",
        }
    }

    pub fn load_preview(&self) -> &'static str {
        match self.lang {
            Language::English => "Load preview",
            Language::German => "Vorschau laden",
        }
    }

    pub fn cancel(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancel",
            Language::German => "Abbrechen",
        }
    }

    pub fn replace_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Replace current library",
            Language::German => "Aktuelle Bibliothek ersetzen",
        }
    }

    pub fn merge_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Merge (keep current conflicts)",
            Language::German => "Zusammenführen (Konflikte behalten)",
        }
    }

    pub fn status_import_merged(
        &self,
        added: usize,
        duplicates: usize,
        conflicts: usize,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Import merged: {added} added, {duplicates} identical duplicates ignored, {conflicts} trigger conflicts kept; {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Import zusammengeführt: {added} hinzugefügt, {duplicates} identische Duplikate ignoriert, {conflicts} Trigger-Konflikte beibehalten; {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn settings_title(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand settings",
            Language::German => "WayExpand-Einstellungen",
        }
    }

    pub fn buffer_limit(&self) -> &'static str {
        match self.lang {
            Language::English => "Matcher buffer limit",
            Language::German => "Matcher-Puffer-Limit",
        }
    }

    pub fn buffer_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Characters retained while looking for a trigger (1–4096).",
            Language::German => {
                "Zeichen, die bei der Suche nach einem Auslöser beibehalten werden (1–4096)."
            }
        }
    }

    pub fn undo_chord(&self) -> &'static str {
        match self.lang {
            Language::English => "Undo shortcut",
            Language::German => "Rückgängig-Tastenkombination",
        }
    }

    pub fn undo_chord_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Pressed right after an expansion with nothing typed in between, reverts it. Leave empty to disable."
            }
            Language::German => {
                "Direkt nach einer Erweiterung gedrückt (ohne dazwischen zu tippen), macht sie rückgängig. Leer lassen zum Deaktivieren."
            }
        }
    }

    pub fn save_settings(&self) -> &'static str {
        match self.lang {
            Language::English => "Save settings",
            Language::German => "Einstellungen speichern",
        }
    }

    pub fn close(&self) -> &'static str {
        match self.lang {
            Language::English => "Close",
            Language::German => "Schließen",
        }
    }

    // Sidebar
    pub fn your_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Your reusable text library",
            Language::German => "Ihre wiederverwendbare Textbibliothek",
        }
    }

    pub fn filtered_snippets(&self) -> &'static str {
        match self.lang {
            Language::English => "Filtered snippets",
            Language::German => "Gefilterte Snippets",
        }
    }

    pub fn new_button(&self) -> &'static str {
        match self.lang {
            Language::English => "+ New",
            Language::German => "+ Neu",
        }
    }

    pub fn new_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Create a new snippet (Ctrl+N)",
            Language::German => "Ein neues Snippet erstellen (Strg+N)",
        }
    }

    pub fn duplicate(&self) -> &'static str {
        match self.lang {
            Language::English => "Duplicate",
            Language::German => "Duplizieren",
        }
    }

    pub fn undo_button(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("Undo ({})", count),
            Language::German => format!("Rückgängig ({})", count),
        }
    }

    pub fn all(&self) -> &'static str {
        match self.lang {
            Language::English => "All",
            Language::German => "Alle",
        }
    }

    pub fn no_snippets(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippets yet.",
            Language::German => "Noch keine Snippets.",
        }
    }

    pub fn no_matches_filter(&self, filter: &str) -> String {
        match self.lang {
            Language::English => format!("No matches for \"{}\".", filter),
            Language::German => format!("Keine Treffer für \"{}\".", filter),
        }
    }

    pub fn no_matches_category(&self, filter: &str, category: &str) -> String {
        match self.lang {
            Language::English => format!("No matches for \"{}\" in {}.", filter, category),
            Language::German => format!("Keine Treffer für \"{}\" in {}.", filter, category),
        }
    }

    pub fn no_snippets_category(&self, category: &str) -> String {
        match self.lang {
            Language::English => format!("No snippets in {}.", category),
            Language::German => format!("Keine Snippets in {}.", category),
        }
    }

    pub fn clear_filters(&self) -> &'static str {
        match self.lang {
            Language::English => "Clear filters",
            Language::German => "Filter löschen",
        }
    }

    // Editor
    pub fn snippet_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet details",
            Language::German => "Snippet-Details",
        }
    }

    pub fn trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Trigger",
            Language::German => "Auslöser",
        }
    }

    pub fn trigger_hint(&self) -> &'static str {
        match self.lang {
            Language::English => ";;hello",
            Language::German => ";;hallo",
        }
    }

    pub fn duplicate_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: another snippet already uses this trigger; saving will be rejected.",
            Language::German => "Warnung: Ein anderes Snippet verwendet bereits diesen Auslöser; das Speichern wird abgelehnt.",
        }
    }

    pub fn trigger_tip(&self) -> &'static str {
        match self.lang {
            Language::English => "Tip: use a distinctive prefix such as ;; or : to avoid accidental matches.",
            Language::German => "Tipp: Verwenden Sie ein eindeutiges Präfix wie ;; oder :, um versehentliche Treffer zu vermeiden.",
        }
    }

    pub fn description(&self) -> &'static str {
        match self.lang {
            Language::English => "Description",
            Language::German => "Beschreibung",
        }
    }

    pub fn tags(&self) -> &'static str {
        match self.lang {
            Language::English => "Tags",
            Language::German => "Tags",
        }
    }

    pub fn category(&self) -> &'static str {
        match self.lang {
            Language::English => "Category",
            Language::German => "Kategorie",
        }
    }

    pub fn category_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "productivity, shortcuts, custom",
            Language::German => "Produktivität, Verknüpfungen, Benutzerdefiniert",
        }
    }

    pub fn existing(&self) -> &'static str {
        match self.lang {
            Language::English => "Existing",
            Language::German => "Vorhandene",
        }
    }

    pub fn app_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "Only in these apps",
            Language::German => "Nur in diesen Apps",
        }
    }

    pub fn detect_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Use current app",
            Language::German => "Aktuelle App verwenden",
        }
    }

    pub fn detect_app_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Detect the app you were last focused on before switching to WayExpand (KDE Plasma only for now)",
            Language::German => "Erkennen Sie die App, auf die Sie sich zuletzt konzentriert haben, bevor Sie zu WayExpand wechseln (nur KDE Plasma)",
        }
    }

    pub fn window_tracking_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: if window tracking isn't available on your compositor, this snippet will never match rather than matching everywhere.",
            Language::German => "Warnung: Wenn die Fenster-Verfolgung in Ihrem Kompositor nicht verfügbar ist, passt dieses Snippet nie, anstatt überall zu passen.",
        }
    }

    pub fn enabled(&self) -> &'static str {
        match self.lang {
            Language::English => "Enabled",
            Language::German => "Aktiviert",
        }
    }

    pub fn immediate(&self) -> &'static str {
        match self.lang {
            Language::English => "Immediate",
            Language::German => "Sofort",
        }
    }

    pub fn word_boundary(&self) -> &'static str {
        match self.lang {
            Language::English => "Word boundary",
            Language::German => "Wortgrenze",
        }
    }

    pub fn propagate_case(&self) -> &'static str {
        match self.lang {
            Language::English => "Match case",
            Language::German => "Groß-/Kleinschreibung anpassen",
        }
    }

    pub fn propagate_case_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Typing the trigger in UPPERCASE or Capitalized form applies the same casing to the replacement"
            }
            Language::German => {
                "Wird der Auslöser in GROSSBUCHSTABEN oder Großschreibung eingegeben, erhält der Ersatztext dieselbe Schreibweise"
            }
        }
    }

    pub fn replacement(&self) -> &'static str {
        match self.lang {
            Language::English => "Replacement",
            Language::German => "Ersatz",
        }
    }

    pub fn save_changes(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes",
            Language::German => "Änderungen speichern",
        }
    }

    pub fn save_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes (Ctrl+S)",
            Language::German => "Änderungen speichern (Strg+S)",
        }
    }

    pub fn delete(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete…",
            Language::German => "Löschen…",
        }
    }

    pub fn preview(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview",
            Language::German => "Vorschau",
        }
    }

    pub fn input(&self) -> &'static str {
        match self.lang {
            Language::English => "Input",
            Language::German => "Eingabe",
        }
    }

    pub fn input_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "text containing the trigger",
            Language::German => "Text mit dem Auslöser",
        }
    }

    pub fn use_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Use trigger",
            Language::German => "Auslöser verwenden",
        }
    }

    pub fn copy(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy",
            Language::German => "Kopieren",
        }
    }

    pub fn copy_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy the previewed output",
            Language::German => "Kopieren Sie die vorbereitete Ausgabe",
        }
    }

    pub fn template_variables(&self) -> &'static str {
        match self.lang {
            Language::English => "Template variables",
            Language::German => "Template-Variablen",
        }
    }

    pub fn template_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert a safe built-in value into the replacement.",
            Language::German => "Fügen Sie einen sicheren integrierten Wert in den Ersatz ein.",
        }
    }

    pub fn dynamic_command(&self) -> &'static str {
        match self.lang {
            Language::English => "Dynamic command (optional)",
            Language::German => "Dynamischer Befehl (optional)",
        }
    }

    pub fn command_checkbox(&self) -> &'static str {
        match self.lang {
            Language::English => "Run a direct program when this snippet matches",
            Language::German => "Führen Sie ein direktes Programm aus, wenn dieses Snippet passt",
        }
    }

    pub fn command_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Only the configured executable is run; shell syntax is never interpreted. Arguments are entered one per line.",
            Language::German => "Nur die konfigurierte ausführbare Datei wird ausgeführt; Shell-Syntax wird nie interpretiert. Argumente werden eine pro Zeile eingegeben.",
        }
    }

    pub fn command_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: this runs a local executable when the trigger matches.",
            Language::German => {
                "Warnung: Dieses Programm wird ausgeführt, wenn der Auslöser passt."
            }
        }
    }

    pub fn program(&self) -> &'static str {
        match self.lang {
            Language::English => "Program",
            Language::German => "Programm",
        }
    }

    pub fn program_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "uname",
            Language::German => "uname",
        }
    }

    pub fn timeout_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Timeout ms",
            Language::German => "Zeitüberschreitung ms",
        }
    }

    pub fn cache_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Cache ms",
            Language::German => "Cache ms",
        }
    }

    pub fn arguments(&self) -> &'static str {
        match self.lang {
            Language::English => "Arguments (one per line)",
            Language::German => "Argumente (eine pro Zeile)",
        }
    }

    pub fn command_backed_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This snippet is command-backed; replacement is stored fallback text."
            }
            Language::German => {
                "Dieses Snippet wird durch Befehl unterstützt; Der Ersatz ist Fallback-Text."
            }
        }
    }

    // Dialogs
    pub fn unsaved_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Unsaved changes",
            Language::German => "Ungespeicherte Änderungen",
        }
    }

    pub fn unsaved_switching(&self) -> &'static str {
        match self.lang {
            Language::English => "switching snippets",
            Language::German => "Snippets wechseln",
        }
    }

    pub fn unsaved_creating(&self) -> &'static str {
        match self.lang {
            Language::English => "creating a snippet",
            Language::German => "ein Snippet erstellen",
        }
    }

    pub fn unsaved_duplicating(&self) -> &'static str {
        match self.lang {
            Language::English => "duplicating a snippet",
            Language::German => "ein Snippet duplizieren",
        }
    }

    pub fn unsaved_deleting(&self) -> &'static str {
        match self.lang {
            Language::English => "deleting a snippet",
            Language::German => "ein Snippet löschen",
        }
    }

    pub fn unsaved_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "reloading the configuration",
            Language::German => "die Konfiguration neu laden",
        }
    }

    pub fn unsaved_undoing(&self) -> &'static str {
        match self.lang {
            Language::English => "undoing the last change",
            Language::German => "die letzte Änderung rückgängig machen",
        }
    }

    pub fn unsaved_closing(&self) -> &'static str {
        match self.lang {
            Language::English => "closing WayExpand",
            Language::German => "WayExpand schließen",
        }
    }

    pub fn save_before(&self, action: &str) -> String {
        match self.lang {
            Language::English => format!("Save changes before {}?", action),
            Language::German => format!("Änderungen vor {} speichern?", action),
        }
    }

    pub fn save_continue(&self) -> &'static str {
        match self.lang {
            Language::English => "Save and continue",
            Language::German => "Speichern und fortfahren",
        }
    }

    pub fn discard(&self) -> &'static str {
        match self.lang {
            Language::English => "Discard",
            Language::German => "Verwerfen",
        }
    }

    pub fn delete_confirm(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Delete this snippet? This cannot be recovered except through Undo."
            }
            Language::German => {
                "Dieses Snippet löschen? Dies kann nur über Rückgängig wiederhergestellt werden."
            }
        }
    }

    pub fn delete_button(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete snippet",
            Language::German => "Snippet löschen",
        }
    }

    // Status messages
    pub fn ready(&self) -> &'static str {
        match self.lang {
            Language::English => "Ready",
            Language::German => "Fertig",
        }
    }

    pub fn new_snippet_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "New snippet draft · complete it and save to add it",
            Language::German => "Neuer Snippet-Entwurf · ausfüllen und speichern zum Hinzufügen",
        }
    }

    pub fn new_snippet_replacement_required(&self) -> &'static str {
        match self.lang {
            Language::English => "A new snippet needs replacement text before it can be saved",
            Language::German => {
                "Ein neues Snippet benötigt Ersatztext, bevor es gespeichert werden kann"
            }
        }
    }

    pub fn no_selection(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippet selected",
            Language::German => "Kein Snippet ausgewählt",
        }
    }

    // Appearance and settings chrome
    pub fn appearance(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance",
            Language::German => "Erscheinungsbild",
        }
    }

    pub fn engine(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing engine",
            Language::German => "Eingabe-Engine",
        }
    }

    pub fn appearance_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance changes apply and are saved immediately.",
            Language::German => {
                "Änderungen am Erscheinungsbild werden sofort übernommen und gespeichert."
            }
        }
    }

    pub fn engine_note(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "These values are stored in your configuration file. Press Save to apply them."
            }
            Language::German => {
                "Diese Werte werden in der Konfigurationsdatei gespeichert. Zum Übernehmen auf Speichern klicken."
            }
        }
    }

    pub fn theme(&self) -> &'static str {
        match self.lang {
            Language::English => "Theme",
            Language::German => "Design",
        }
    }

    pub fn theme_dark(&self) -> &'static str {
        match self.lang {
            Language::English => "Dark",
            Language::German => "Dunkel",
        }
    }

    pub fn theme_light(&self) -> &'static str {
        match self.lang {
            Language::English => "Light",
            Language::German => "Hell",
        }
    }

    pub fn language(&self) -> &'static str {
        match self.lang {
            Language::English => "Language",
            Language::German => "Sprache",
        }
    }

    pub fn color_pack(&self) -> &'static str {
        match self.lang {
            Language::English => "Color pack",
            Language::German => "Farbschema",
        }
    }

    pub fn color_pack_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Pick a visual style for the editor.",
            Language::German => "Visuellen Stil für den Editor auswählen.",
        }
    }

    pub fn font_size(&self) -> &'static str {
        match self.lang {
            Language::English => "Font size",
            Language::German => "Schriftgröße",
        }
    }

    pub fn font_size_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Adjust text size for readability on your display.",
            Language::German => "Textgröße für die Lesbarkeit auf Ihrem Bildschirm anpassen.",
        }
    }

    pub fn font_scale_label(&self, scale: FontScale) -> &'static str {
        match (self.lang, scale) {
            (Language::English, FontScale::Small) => "Small (80%)",
            (Language::English, FontScale::Normal) => "Normal (100%)",
            (Language::English, FontScale::Large) => "Large (120%)",
            (Language::English, FontScale::ExtraLarge) => "Extra large (150%)",
            (Language::English, FontScale::Huge) => "Huge (200%)",
            (Language::German, FontScale::Small) => "Klein (80 %)",
            (Language::German, FontScale::Normal) => "Normal (100 %)",
            (Language::German, FontScale::Large) => "Groß (120 %)",
            (Language::German, FontScale::ExtraLarge) => "Sehr groß (150 %)",
            (Language::German, FontScale::Huge) => "Riesig (200 %)",
        }
    }

    // Editor chrome
    pub fn preview_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview app",
            Language::German => "Vorschau-App",
        }
    }

    pub fn preview_app_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "leave empty for no focused app",
            Language::German => "leer lassen für keine fokussierte App",
        }
    }

    pub fn run_once(&self) -> &'static str {
        match self.lang {
            Language::English => "Run once",
            Language::German => "Einmal ausführen",
        }
    }

    pub fn running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running…",
            Language::German => "Läuft…",
        }
    }

    pub fn command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This snippet runs a program instead of inserting fixed text. Its output is not shown automatically — run it once to see what it currently produces."
            }
            Language::German => {
                "Dieses Snippet führt ein Programm aus, statt festen Text einzufügen. Die Ausgabe wird nicht automatisch angezeigt — einmal ausführen, um das aktuelle Ergebnis zu sehen."
            }
        }
    }

    pub fn detecting_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Detecting focused application…",
            Language::German => "Fokussierte Anwendung wird erkannt…",
        }
    }

    pub fn stopping_app_detection(&self) -> &'static str {
        match self.lang {
            Language::English => "Stopping application detection…",
            Language::German => "Anwendungserkennung wird beendet…",
        }
    }

    pub fn no_description(&self) -> &'static str {
        match self.lang {
            Language::English => "No description",
            Language::German => "Keine Beschreibung",
        }
    }

    pub fn click_to_enable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to enable",
            Language::German => "Zum Aktivieren klicken",
        }
    }

    pub fn click_to_disable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to disable",
            Language::German => "Zum Deaktivieren klicken",
        }
    }

    pub fn selection_stale(&self) -> &'static str {
        match self.lang {
            Language::English => "Selection is out of date; choose a snippet again.",
            Language::German => "Auswahl ist veraltet; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn draft_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet draft unavailable; choose a snippet again.",
            Language::German => "Snippet-Entwurf nicht verfügbar; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn no_snippets_match_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippets match this filter.",
            Language::German => "Keine Snippets entsprechen diesem Filter.",
        }
    }

    pub fn configuration_file(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration",
            Language::German => "Konfiguration",
        }
    }

    // Diagnostics
    pub fn fleet_layers(&self) -> &'static str {
        match self.lang {
            Language::English => "Fleet layers",
            Language::German => "Flotten-Ebenen",
        }
    }

    pub fn not_checked(&self) -> &'static str {
        match self.lang {
            Language::English => "Not checked",
            Language::German => "Nicht geprüft",
        }
    }

    /// A readable label for a backend's state. The raw
    /// `implementation/availability/permission` triple is still shown
    /// verbatim next to it so the GUI stays a faithful mirror of
    /// `wayexpand doctor` rather than paraphrasing it away.
    pub fn backend_state(&self, state: BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English, BackendState::Available) => "Ready",
            (Language::English, BackendState::RequiresPermission) => "Needs permission",
            (Language::English, BackendState::Implemented) => "Not probed",
            (Language::English, BackendState::Unavailable) => "Unavailable",
            (Language::English, BackendState::NotImplemented) => "Not implemented",
            (Language::German, BackendState::Available) => "Bereit",
            (Language::German, BackendState::RequiresPermission) => "Berechtigung nötig",
            (Language::German, BackendState::Implemented) => "Nicht geprüft",
            (Language::German, BackendState::Unavailable) => "Nicht verfügbar",
            (Language::German, BackendState::NotImplemented) => "Nicht implementiert",
        }
    }

    // Import
    pub fn import_preview_summary(
        &self,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Migration preview: {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Importvorschau: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    // Status line
    pub fn status_diagnostics_refreshed(&self) -> &'static str {
        match self.lang {
            Language::English => "Diagnostics refreshed",
            Language::German => "Diagnose aktualisiert",
        }
    }

    pub fn diagnostics_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running diagnostics…",
            Language::German => "Diagnose läuft…",
        }
    }

    pub fn daemon_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "Reloading daemon configuration…",
            Language::German => "Daemon-Konfiguration wird neu geladen…",
        }
    }

    pub fn config_reload_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Loading configuration…",
            Language::German => "Konfiguration wird geladen…",
        }
    }

    pub fn status_reload_discarded_due_edits(&self) -> &'static str {
        match self.lang {
            Language::English => "Reload discarded because the configuration or draft changed while loading",
            Language::German => "Neuladen verworfen, da sich Konfiguration oder Entwurf während des Ladens geändert hat",
        }
    }

    pub fn daemon_control_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Updating daemon state…",
            Language::German => "Daemon-Status wird aktualisiert…",
        }
    }

    pub fn background_queue_full(&self) -> &'static str {
        match self.lang {
            Language::English => "Background task queue is busy; try again shortly",
            Language::German => {
                "Hintergrundwarteschlange ausgelastet; bitte gleich erneut versuchen"
            }
        }
    }

    pub fn background_runtime_stopped(&self) -> &'static str {
        match self.lang {
            Language::English => "Background runtime stopped unexpectedly",
            Language::German => "Hintergrunddienst wurde unerwartet beendet",
        }
    }

    pub fn status_daemon_not_reloaded(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("daemon did not reload: {detail}"),
            Language::German => format!("Daemon hat nicht neu geladen: {detail}"),
        }
    }

    pub fn status_buffer_limit_not_a_number(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("buffer limit must be a whole number ({detail})"),
            Language::German => format!("Puffergrenze muss eine ganze Zahl sein ({detail})"),
        }
    }

    pub fn status_settings_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings invalid: {detail}"),
            Language::German => format!("Einstellungen ungültig: {detail}"),
        }
    }

    pub fn status_settings_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings rejected: {detail}"),
            Language::German => format!("Einstellungen abgelehnt: {detail}"),
        }
    }

    pub fn status_settings_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Settings saved atomically",
            Language::German => "Einstellungen atomar gespeichert",
        }
    }

    pub fn status_appearance_save_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance preference could not be saved",
            Language::German => "Darstellungseinstellung konnte nicht gespeichert werden",
        }
    }

    pub fn status_settings_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings save failed: {detail}"),
            Language::German => format!("Speichern der Einstellungen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_font_size_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Font size saved",
            Language::German => "Schriftgröße gespeichert",
        }
    }

    pub fn status_import_loaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Espanso library loaded for review",
            Language::German => "Espanso-Bibliothek zur Prüfung geladen",
        }
    }

    pub fn status_import_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import failed: {detail}"),
            Language::German => format!("Import fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_import_needs_clean_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "Save or discard the current draft before importing",
            Language::German => "Aktuellen Entwurf vor dem Import speichern oder verwerfen",
        }
    }

    pub fn status_import_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import rejected: {detail}"),
            Language::German => format!("Import abgelehnt: {detail}"),
        }
    }

    pub fn status_import_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import save failed: {detail}"),
            Language::German => format!("Speichern des Imports fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_imported_with_report(
        &self,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Espanso import applied: {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Espanso-Import angewendet: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn status_config_reloaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration reloaded",
            Language::German => "Konfiguration neu geladen",
        }
    }

    pub fn status_config_changed_externally(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration changed outside WayExpand; reload it before saving",
            Language::German => {
                "Konfiguration wurde außerhalb von WayExpand geändert; vor dem Speichern neu laden"
            }
        }
    }

    pub fn status_reload_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Reload failed: {detail}"),
            Language::German => format!("Neu laden fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command settings invalid: {detail}"),
            Language::German => format!("Befehlseinstellungen ungültig: {detail}"),
        }
    }

    pub fn status_save_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save rejected: {detail}"),
            Language::German => format!("Speichern abgelehnt: {detail}"),
        }
    }

    pub fn status_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save failed: {detail}"),
            Language::German => format!("Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_snippet_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet saved atomically",
            Language::German => "Snippet atomar gespeichert",
        }
    }

    pub fn status_nothing_to_undo(&self) -> &'static str {
        match self.lang {
            Language::English => "Nothing to undo",
            Language::German => "Nichts zum Rückgängigmachen",
        }
    }

    pub fn status_undo_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Undo save failed: {detail}"),
            Language::German => format!("Rückgängig-Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_undone(&self) -> &'static str {
        match self.lang {
            Language::English => "Undid the last saved change",
            Language::German => "Letzte gespeicherte Änderung rückgängig gemacht",
        }
    }

    pub fn status_created(&self) -> &'static str {
        match self.lang {
            Language::English => "Created a new snippet",
            Language::German => "Neues Snippet erstellt",
        }
    }

    pub fn status_create_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Create failed: {detail}"),
            Language::German => format!("Erstellen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_duplicated(&self) -> &'static str {
        match self.lang {
            Language::English => "Duplicated snippet",
            Language::German => "Snippet dupliziert",
        }
    }

    pub fn status_duplicate_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Duplicate failed: {detail}"),
            Language::German => format!("Duplizieren fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_deleted(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Deleted {trigger}"),
            Language::German => format!("{trigger} gelöscht"),
        }
    }

    pub fn status_delete_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Delete failed: {detail}"),
            Language::German => format!("Löschen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_snippet_enabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} enabled"),
            Language::German => format!("{trigger} aktiviert"),
        }
    }

    pub fn status_snippet_disabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} disabled"),
            Language::German => format!("{trigger} deaktiviert"),
        }
    }

    pub fn status_toggle_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Toggle failed: {detail}"),
            Language::German => format!("Umschalten fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_paused(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion paused",
            Language::German => "Erweiterung angehalten",
        }
    }

    pub fn status_resumed(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion resumed",
            Language::German => "Erweiterung fortgesetzt",
        }
    }

    pub fn status_control_unavailable(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Daemon control unavailable: {detail}"),
            Language::German => format!("Daemon-Steuerung nicht verfügbar: {detail}"),
        }
    }

    pub fn status_app_filter_added(&self, value: &str) -> String {
        match self.lang {
            Language::English => format!("Added \"{value}\" to the app filter"),
            Language::German => format!("\"{value}\" zum App-Filter hinzugefügt"),
        }
    }

    pub fn status_window_unidentified(&self) -> &'static str {
        match self.lang {
            Language::English => "Could not identify the focused window",
            Language::German => "Fokussiertes Fenster konnte nicht ermittelt werden",
        }
    }

    pub fn status_no_focused_window(&self) -> &'static str {
        match self.lang {
            Language::English => "No focused window to detect (focus is on the desktop)",
            Language::German => {
                "Kein fokussiertes Fenster erkennbar (Fokus liegt auf der Arbeitsfläche)"
            }
        }
    }

    pub fn status_detection_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection is unavailable here (KDE Plasma only for now)",
            Language::German => {
                "Fenstererkennung ist hier nicht verfügbar (derzeit nur KDE Plasma)"
            }
        }
    }

    pub fn status_detection_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection failed unexpectedly",
            Language::German => "Fenstererkennung ist unerwartet fehlgeschlagen",
        }
    }

    pub fn status_preview_copied(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview copied to clipboard",
            Language::German => "Vorschau in die Zwischenablage kopiert",
        }
    }

    pub fn status_enable_command_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable the dynamic command first",
            Language::German => "Zuerst den dynamischen Befehl aktivieren",
        }
    }

    pub fn status_command_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command failed: {detail}"),
            Language::German => format!("Befehl fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_preview_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Command preview failed unexpectedly",
            Language::German => "Befehlsvorschau ist unerwartet fehlgeschlagen",
        }
    }

    pub fn search_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Search triggers, descriptions, tags, and categories (Ctrl+F)",
            Language::German => {
                "Trigger, Beschreibungen, Schlagwörter und Kategorien durchsuchen (Strg+F)"
            }
        }
    }

    pub fn command_backed_summary(&self) -> &'static str {
        match self.lang {
            Language::English => "Command-backed snippet",
            Language::German => "Befehlsgestütztes Snippet",
        }
    }

    pub fn app_filter_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Comma separated. Currently works only with the KWin window tracker; elsewhere filtered snippets fail closed. Leave empty to expand in every application.",
            Language::German => "Kommagetrennt. Derzeit nur mit dem KWin-Fenster-Tracker verfügbar; sonst werden gefilterte Snippets nicht erweitert. Leer lassen, um in jeder Anwendung zu erweitern.",
        }
    }

    pub fn matching(&self) -> &'static str {
        match self.lang {
            Language::English => "Matching",
            Language::German => "Erkennung",
        }
    }
}
