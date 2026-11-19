use super::{Language, RouteTrustDetails, Strings};
use crate::*;

/// The one-word state shown in the header's single status chip. Details
/// (service, typing route, protection, replacement) live in its tooltip and
/// in diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderState {
    /// Expansion confirmed in a real application on this route and desktop.
    Working,
    /// Confirmed, but the confirmation could not be saved: this session only.
    WorkingUnsaved,
    /// Connected with nothing wrong, but no expansion confirmed yet.
    NeedsVerification,
    Limited,
    Paused,
    Off,
    NotSetUp,
    Checking,
    Starting,
    Reconnecting,
    NeedsPermission,
    /// Automatic replacement is unavailable; the Quick Picker still works.
    CopyOnly,
    Stopped,
    /// A route that was available failed.
    Disconnected,
    /// Running, but a setup step remains (input source, restart, consent).
    NeedsSetup,
}

impl Strings {
    pub fn header_state(&self, state: HeaderState) -> &'static str {
        use HeaderState as S;
        match (self.lang, state) {
            (Language::English, S::Working) => "Working",
            (Language::English, S::WorkingUnsaved) => "Working (this session only)",
            (Language::English, S::NeedsVerification) => "Needs verification",
            (Language::English, S::Limited) => "Limited",
            (Language::English, S::Paused) => "Paused",
            (Language::English, S::Off) => "Off",
            (Language::English, S::NotSetUp) => "Not set up",
            (Language::English, S::Checking) => "Checking…",
            (Language::English, S::Starting) => "Starting…",
            (Language::English, S::Reconnecting) => "Reconnecting…",
            (Language::English, S::NeedsPermission) => "Needs permission",
            (Language::English, S::CopyOnly) => "Copy-only",
            (Language::English, S::Stopped) => "Stopped",
            (Language::English, S::Disconnected) => "Disconnected",
            (Language::English, S::NeedsSetup) => "Needs setup",
            (Language::German, S::Working) => "Funktioniert",
            (Language::German, S::WorkingUnsaved) => "Funktioniert (nur diese Sitzung)",
            (Language::German, S::NeedsVerification) => "Prüfung nötig",
            (Language::German, S::Limited) => "Eingeschränkt",
            (Language::German, S::Paused) => "Pausiert",
            (Language::German, S::Off) => "Aus",
            (Language::German, S::NotSetUp) => "Nicht eingerichtet",
            (Language::German, S::Checking) => "Wird geprüft…",
            (Language::German, S::Starting) => "Startet…",
            (Language::German, S::Reconnecting) => "Verbindet neu…",
            (Language::German, S::NeedsPermission) => "Berechtigung nötig",
            (Language::German, S::CopyOnly) => "Nur Kopieren",
            (Language::German, S::Stopped) => "Gestoppt",
            (Language::German, S::Disconnected) => "Getrennt",
            (Language::German, S::NeedsSetup) => "Einrichtung nötig",
            (Language::Indonesian, S::Working) => "Berfungsi",
            (Language::Indonesian, S::WorkingUnsaved) => "Berfungsi (sesi ini saja)",
            (Language::Indonesian, S::NeedsVerification) => "Perlu verifikasi",
            (Language::Indonesian, S::Limited) => "Terbatas",
            (Language::Indonesian, S::Paused) => "Dijeda",
            (Language::Indonesian, S::Off) => "Mati",
            (Language::Indonesian, S::NotSetUp) => "Belum disiapkan",
            (Language::Indonesian, S::Checking) => "Memeriksa…",
            (Language::Indonesian, S::Starting) => "Memulai…",
            (Language::Indonesian, S::Reconnecting) => "Menyambung ulang…",
            (Language::Indonesian, S::NeedsPermission) => "Perlu izin",
            (Language::Indonesian, S::CopyOnly) => "Salin saja",
            (Language::Indonesian, S::Stopped) => "Berhenti",
            (Language::Indonesian, S::Disconnected) => "Terputus",
            (Language::Indonesian, S::NeedsSetup) => "Perlu penyiapan",
        }
    }

    pub fn snippets_count(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("{} snippets", count),
            Language::Indonesian => format!("{} cuplikan", count),
            Language::German => format!("{} Snippets", count),
        }
    }

    pub fn hotkeys_count(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("{} hotkeys", count),
            Language::Indonesian => format!("{} tombol pintas", count),
            Language::German => format!("{} Hotkeys", count),
        }
    }

    pub fn sync_save_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Save or discard the open edit before syncing",
            Language::Indonesian => {
                "Simpan atau buang suntingan yang terbuka sebelum menyinkronkan"
            }
            Language::German => {
                "Vor dem Synchronisieren die offene Änderung speichern oder verwerfen"
            }
        }
    }

    pub fn route_trust_status(&self, details: RouteTrustDetails<'_>) -> String {
        let privacy = match (self.lang, details.sensitive_fields) {
            (Language::English, Some(true)) => "sensitive-field signal available",
            (Language::English, Some(false)) => "no sensitive-field protection",
            (Language::English, None) => "sensitive-field protection unknown",
            (Language::Indonesian, Some(true)) => "sinyal kolom sensitif tersedia",
            (Language::Indonesian, Some(false)) => "tanpa perlindungan kolom sensitif",
            (Language::Indonesian, None) => "perlindungan kolom sensitif tidak diketahui",
            (Language::German, Some(true)) => "Signal für sensible Felder verfügbar",
            (Language::German, Some(false)) => "kein Schutz sensibler Felder",
            (Language::German, None) => "Schutz sensibler Felder unbekannt",
        };
        let precondition = match (self.lang, details.trigger_precondition) {
            (Language::English, "surrounding_text_matches_trigger") => {
                "trigger checked against surrounding text"
            }
            (Language::English, "focus_checked_with_compatibility_consent") => {
                "focus checked with compatibility consent"
            }
            (Language::English, "none") => "trigger target not verified",
            (Language::English, _) => "trigger precondition unknown",
            (Language::Indonesian, "surrounding_text_matches_trigger") => {
                "pemicu dicocokkan dengan teks sekitar"
            }
            (Language::Indonesian, "focus_checked_with_compatibility_consent") => {
                "fokus diperiksa dengan izin kompatibilitas"
            }
            (Language::Indonesian, "none") => "tujuan pemicu tidak terverifikasi",
            (Language::Indonesian, _) => "prasyarat pemicu tidak diketahui",
            (Language::German, "surrounding_text_matches_trigger") => {
                "Auslöser mit umgebendem Text abgeglichen"
            }
            (Language::German, "focus_checked_with_compatibility_consent") => {
                "Fokus mit Kompatibilitätsfreigabe geprüft"
            }
            (Language::German, "none") => "Auslöserziel nicht verifiziert",
            (Language::German, _) => "Auslöservoraussetzung unbekannt",
        };
        let transaction = match (self.lang, details.output_transaction) {
            (Language::English, "separate_delete_and_commit") => "non-atomic delete and commit",
            (Language::English, "separate_delete_and_key_events") => {
                "non-atomic delete and key events"
            }
            (Language::English, "atomic_protocol_batch") => "atomic protocol batch",
            (Language::English, _) => "output transaction unknown",
            (Language::Indonesian, "separate_delete_and_commit") => {
                "penghapusan dan komit non-atomik"
            }
            (Language::Indonesian, "separate_delete_and_key_events") => {
                "penghapusan dan tombol non-atomik"
            }
            (Language::Indonesian, "atomic_protocol_batch") => "batch protokol atomik",
            (Language::Indonesian, _) => "transaksi keluaran tidak diketahui",
            (Language::German, "separate_delete_and_commit") => {
                "nicht atomisches Löschen und Übernehmen"
            }
            (Language::German, "separate_delete_and_key_events") => {
                "nicht atomisches Löschen und Tastenereignisse"
            }
            (Language::German, "atomic_protocol_batch") => "atomarer Protokollbatch",
            (Language::German, _) => "Ausgabetransaktion unbekannt",
        };
        let application = match (self.lang, details.application_result) {
            (Language::English, "unobservable") => "application result unverified",
            (Language::Indonesian, "unobservable") => "hasil aplikasi belum terverifikasi",
            (Language::German, "unobservable") => "Anwendungsergebnis unverifiziert",
            (Language::English, _) => "application result unknown",
            (Language::Indonesian, _) => "hasil aplikasi tidak diketahui",
            (Language::German, _) => "Anwendungsergebnis unbekannt",
        };
        let evidence = match (self.lang, details.certified) {
            (Language::English, true) => "certified",
            (Language::Indonesian, true) => "tersertifikasi",
            (Language::English, false) => "certification pending",
            (Language::Indonesian, false) => "sertifikasi tertunda",
            (Language::German, true) => "zertifiziert",
            (Language::German, false) => "Zertifizierung ausstehend",
        };
        format!(
            "{} · {precondition} · {transaction} · {application} · {} · {privacy} · {evidence}",
            details.route, details.maturity
        )
    }

    pub fn backend_label(&self, kind: wayexpand_core::BackendKind) -> &'static str {
        match (self.lang, kind) {
            (
                Language::English,
                wayexpand_core::BackendKind::InputMethodV2 | wayexpand_core::BackendKind::Evdev,
            ) => "Keyboard capture",
            (
                Language::Indonesian,
                wayexpand_core::BackendKind::InputMethodV2 | wayexpand_core::BackendKind::Evdev,
            ) => "Tangkapan papan ketik",
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
                Language::Indonesian,
                wayexpand_core::BackendKind::Libei
                | wayexpand_core::BackendKind::WlrootsVirtualKeyboard
                | wayexpand_core::BackendKind::Uinput,
            ) => "Injeksi teks",
            (
                Language::German,
                wayexpand_core::BackendKind::Libei
                | wayexpand_core::BackendKind::WlrootsVirtualKeyboard
                | wayexpand_core::BackendKind::Uinput,
            ) => "Texteingabe",
            (Language::English, wayexpand_core::BackendKind::WindowTracker) => {
                "Application detection"
            }
            (Language::Indonesian, wayexpand_core::BackendKind::WindowTracker) => {
                "Deteksi aplikasi"
            }
            (Language::German, wayexpand_core::BackendKind::WindowTracker) => "App-Erkennung",
            (Language::English, wayexpand_core::BackendKind::Clipboard) => "Clipboard",
            (Language::Indonesian, wayexpand_core::BackendKind::Clipboard) => "Papan klip",
            (Language::German, wayexpand_core::BackendKind::Clipboard) => "Zwischenablage",
        }
    }

    pub fn onboarding_install_summary(&self, version: &str) -> String {
        match self.lang {
            Language::English => format!("WayExpand GUI {version} is running. The next step starts its user service."),
            Language::Indonesian => format!("WayExpand GUI {version} sedang berjalan. Langkah berikutnya menjalankan layanan pengguna."),
            Language::German => format!("WayExpand GUI {version} läuft. Im nächsten Schritt wird der Benutzerdienst gestartet."),
        }
    }

    /// Short, localized descriptions of the typing route's live state, used
    /// inside the first-run session sentence.
    pub fn onboarding_route_detail(&self, key: &str) -> &'static str {
        match (key, self.lang) {
            ("permission_required", Language::English) => "permission required",
            ("permission_required", Language::Indonesian) => "izin diperlukan",
            ("permission_required", Language::German) => "Berechtigung erforderlich",
            ("portal_revoked", Language::English) => "desktop permission was revoked",
            ("portal_revoked", Language::Indonesian) => "izin desktop dicabut",
            ("portal_revoked", Language::German) => "Desktop-Berechtigung wurde widerrufen",
            ("unsupported", Language::English) => "no supported desktop typing option is available",
            ("unsupported", Language::Indonesian) => {
                "opsi pengetikan desktop yang didukung tidak tersedia"
            }
            ("unsupported", Language::German) => {
                "keine unterstützte Texteingabe am Desktop verfügbar"
            }
            ("failed", Language::English) => "WayExpand could not start desktop typing",
            ("failed", Language::Indonesian) => "WayExpand tidak dapat memulai pengetikan desktop",
            ("failed", Language::German) => {
                "WayExpand konnte die Texteingabe am Desktop nicht starten"
            }
            ("reconnecting", Language::English) => "reconnecting to desktop typing",
            ("reconnecting", Language::Indonesian) => "menyambungkan kembali pengetikan desktop",
            ("reconnecting", Language::German) => "Texteingabe am Desktop wird neu verbunden",
            ("starting", Language::English) => "starting desktop typing",
            ("starting", Language::Indonesian) => "memulai pengetikan desktop",
            ("starting", Language::German) => "Texteingabe am Desktop wird gestartet",
            ("degraded", Language::English) => "desktop typing is not working correctly",
            ("degraded", Language::Indonesian) => "pengetikan desktop tidak berfungsi dengan benar",
            ("degraded", Language::German) => "Texteingabe am Desktop funktioniert nicht richtig",
            ("stopped", Language::English) => "desktop typing is stopped",
            ("stopped", Language::Indonesian) => "pengetikan desktop dihentikan",
            ("stopped", Language::German) => "Texteingabe am Desktop ist gestoppt",
            ("paused", Language::English) => "WayExpand is paused",
            ("paused", Language::Indonesian) => "WayExpand dijeda",
            ("paused", Language::German) => "WayExpand ist pausiert",
            ("service_not_running", Language::English) => "WayExpand is not running yet",
            ("service_not_running", Language::Indonesian) => "WayExpand belum berjalan",
            ("service_not_running", Language::German) => "WayExpand läuft noch nicht",
            ("waiting", Language::English) => "checking desktop typing",
            ("waiting", Language::Indonesian) => "memeriksa pengetikan desktop",
            ("waiting", Language::German) => "Texteingabe am Desktop wird geprüft",
            ("typing_ready", Language::English) => "integration is configured",
            ("typing_ready", Language::Indonesian) => "integrasi telah dikonfigurasi",
            ("typing_ready", Language::German) => "Integration ist eingerichtet",
            ("terminal_only_ready", Language::English) => {
                "configured for the selected terminal apps only"
            }
            ("terminal_only_ready", Language::Indonesian) => {
                "hanya untuk aplikasi terminal yang dipilih"
            }
            ("terminal_only_ready", Language::German) => {
                "nur für die ausgewählten Terminal-Apps eingerichtet"
            }
            _ => "",
        }
    }

    pub fn onboarding_session_state(
        &self,
        ready: bool,
        detail: &str,
        daemon_build: Option<crate::runtime::DaemonBuildState>,
        restart_command: Option<&str>,
    ) -> String {
        match daemon_build {
            Some(crate::runtime::DaemonBuildState::RestartRequired) => {
                return self.onboarding_stale_daemon_state(restart_command);
            }
            Some(crate::runtime::DaemonBuildState::Unverified) => {
                return self.onboarding_unverified_daemon_state().to_owned();
            }
            Some(crate::runtime::DaemonBuildState::Current) | None => {}
        }
        match (self.lang, ready) {
            (Language::English, true) => format!("Connected in this session: {detail}. This is a live status check, not certification."),
            (Language::Indonesian, true) => format!("Terhubung di sesi ini: {detail}. Ini pemeriksaan status langsung, bukan sertifikasi."),
            (Language::German, true) => format!("In dieser Sitzung verbunden: {detail}. Dies ist eine Live-Statusprüfung, keine Zertifizierung."),
            (Language::English, false) => format!("Not ready yet: {detail}. Refresh checks after granting desktop permissions or starting the service."),
            (Language::Indonesian, false) => format!("Belum siap: {detail}. Segarkan pemeriksaan setelah memberi izin desktop atau menjalankan layanan."),
            (Language::German, false) => format!("Noch nicht bereit: {detail}. Aktualisieren Sie nach der Desktop-Freigabe oder dem Start des Dienstes."),
        }
    }

    pub fn onboarding_stale_daemon_state(&self, restart_command: Option<&str>) -> String {
        match (self.lang, restart_command) {
            (Language::English, Some(command)) => format!(
                "Not ready: the daemon is running a different build. Run `{command}`, then refresh these checks before testing snippets."
            ),
            (Language::Indonesian, Some(command)) => format!(
                "Belum siap: daemon menjalankan build yang berbeda. Jalankan `{command}`, lalu segarkan pemeriksaan ini sebelum menguji cuplikan."
            ),
            (Language::German, Some(command)) => format!(
                "Noch nicht bereit: Der Daemon verwendet einen anderen Build. Führen Sie `{command}` aus und aktualisieren Sie diese Prüfungen, bevor Sie Snippets testen."
            ),
            (Language::English, None) => "Not ready: the daemon is running a different build. Restart the WayExpand service for this route, then refresh these checks before testing snippets.".into(),
            (Language::Indonesian, None) => "Belum siap: daemon menjalankan build yang berbeda. Mulai ulang layanan WayExpand untuk jalur ini, lalu segarkan pemeriksaan sebelum menguji cuplikan.".into(),
            (Language::German, None) => "Noch nicht bereit: Der Daemon verwendet einen anderen Build. Starten Sie den WayExpand-Dienst für diese Route neu und aktualisieren Sie diese Prüfungen, bevor Sie Snippets testen.".into(),
        }
    }

    pub fn empty_library_prompt(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Your library is empty. Create or import a snippet to get started."
            }
            Language::Indonesian => "Pustaka Anda kosong. Buat atau impor cuplikan untuk memulai.",
            Language::German => {
                "Ihre Bibliothek ist leer. Erstellen oder importieren Sie ein Snippet."
            }
        }
    }

    pub fn onboarding_desktop(&self, desktop: &str, wayland: bool) -> String {
        match (self.lang, wayland) {
            (Language::English, true) => {
                format!("{desktop} · Wayland session detected")
            }
            (Language::Indonesian, true) => {
                format!("{desktop} · sesi Wayland terdeteksi")
            }
            (Language::English, false) => {
                format!("{desktop} · Wayland session not detected")
            }
            (Language::Indonesian, false) => {
                format!("{desktop} · sesi Wayland tidak terdeteksi")
            }
            (Language::German, true) => format!("{desktop} · Wayland-Sitzung erkannt"),
            (Language::German, false) => format!("{desktop} · Keine Wayland-Sitzung erkannt"),
        }
    }

    /// Plain-language route state for the primary first-run step. Detailed
    /// backend and protocol evidence belongs in Diagnostics, not in the
    /// first thing a desktop user sees.
    pub fn onboarding_integration_state(
        &self,
        checking: bool,
        route_ready: bool,
        recommendation: Option<RouteRecommendation>,
    ) -> &'static str {
        use BackendState as S;

        if checking {
            return match self.lang {
                Language::English => "Checking desktop integration…",
                Language::German => "Desktop-Integration wird geprüft …",
                Language::Indonesian => "Memeriksa integrasi desktop…",
            };
        }
        if route_ready {
            return match self.lang {
                Language::English => "Integration connected",
                Language::German => "Integration verbunden",
                Language::Indonesian => "Integrasi terhubung",
            };
        }

        let Some(recommendation) = recommendation else {
            return match self.lang {
                Language::English => "Desktop integration status is not known yet",
                Language::German => "Der Status der Desktop-Integration ist noch unbekannt",
                Language::Indonesian => "Status integrasi desktop belum diketahui",
            };
        };
        if recommendation.blocked_by_policy {
            return match self.lang {
                Language::English => "The available integration is blocked by organization policy",
                Language::German => {
                    "Die verfügbare Integration wird durch die Organisationsrichtlinie blockiert"
                }
                Language::Indonesian => {
                    "Integrasi yang tersedia diblokir oleh kebijakan organisasi"
                }
            };
        }
        if recommendation.capture_state == S::RequiresPermission
            || recommendation.injection_state == S::RequiresPermission
        {
            return match self.lang {
                Language::English => "Desktop permission is needed",
                Language::German => "Eine Desktop-Berechtigung ist erforderlich",
                Language::Indonesian => "Izin desktop diperlukan",
            };
        }
        if matches!(
            recommendation.capture_state,
            S::Unavailable | S::NotImplemented
        ) || matches!(
            recommendation.injection_state,
            S::Unavailable | S::NotImplemented
        ) {
            return match self.lang {
                Language::English => "The recommended integration is not available in this session",
                Language::German => {
                    "Die empfohlene Integration ist in dieser Sitzung nicht verfügbar"
                }
                Language::Indonesian => "Integrasi yang disarankan tidak tersedia di sesi ini",
            };
        }
        if recommendation.capture_state == S::ActionRequired
            || recommendation.injection_state == S::ActionRequired
        {
            return match self.lang {
                Language::English => "Integration setup is needed",
                Language::German => "Die Integration muss eingerichtet werden",
                Language::Indonesian => "Integrasi perlu disiapkan",
            };
        }
        if recommendation.capture_state == S::Available
            && recommendation.injection_state == S::Available
        {
            return match self.lang {
                Language::English => "Integration is available to set up",
                Language::German => "Die Integration kann eingerichtet werden",
                Language::Indonesian => "Integrasi tersedia untuk disiapkan",
            };
        }
        match self.lang {
            Language::English => "Checking integration support",
            Language::German => "Integrationsunterstützung wird geprüft",
            Language::Indonesian => "Memeriksa dukungan integrasi",
        }
    }

    pub fn onboarding_app_caveat(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Application filters depend on desktop integration and may be unavailable. IBus can replace text only when a field provides verifiable surrounding text; use Quick Picker to copy and paste in fields that do not."
            }
            Language::Indonesian => {
                "Filter aplikasi bergantung pada integrasi desktop dan mungkin tidak tersedia. IBus hanya dapat mengganti teks jika kolom menyediakan teks sekitar yang dapat diverifikasi; gunakan Quick Picker untuk menyalin dan menempel di kolom lain."
            }
            Language::German => {
                "App-Filter benötigen Desktop-Integration und sind möglicherweise nicht verfügbar. IBus kann Text nur ersetzen, wenn ein Feld verifizierbaren Umgebungstext bereitstellt; in anderen Feldern können Sie mit dem Schnellwähler kopieren und einfügen."
            }
        }
    }

    pub fn onboarding_test_state_save_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("The test snippet was added, but its onboarding state could not be saved: {error}"),
            Language::Indonesian => format!("Cuplikan uji ditambahkan, tetapi status penyiapannya tidak dapat disimpan: {error}"),
            Language::German => format!("Das Test-Snippet wurde hinzugefügt, aber der Einrichtungsstatus konnte nicht gespeichert werden: {error}"),
        }
    }

    pub fn onboarding_certification_note(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Manual verification only · this desktop is not automatically certified."
            }
            Language::Indonesian => {
                "Hanya verifikasi manual · desktop ini tidak tersertifikasi secara otomatis."
            }
            Language::German => {
                "Nur manuell geprüft · dieser Desktop ist nicht automatisiert zertifiziert."
            }
        }
    }

    pub fn onboarding_detection(&self, available: bool) -> &'static str {
        match (self.lang, available) {
            (Language::English, true) => "Application detection probe available",
            (Language::Indonesian, true) => "Pemeriksaan deteksi aplikasi tersedia",
            (Language::English, false) => "Application detection unavailable on this desktop",
            (Language::Indonesian, false) => "Deteksi aplikasi tidak tersedia di desktop ini",
            (Language::German, true) => "Prüfung der App-Erkennung verfügbar",
            (Language::German, false) => "App-Erkennung auf diesem Desktop nicht verfügbar",
        }
    }

    pub fn onboarding_recommended_route(
        &self,
        recommendation: Option<RouteRecommendation>,
    ) -> (String, String) {
        let Some(route) = recommendation else {
            return match self.lang {
                Language::English => (
                    "No usable route was detected".into(),
                    "Open Diagnostics to see whether a protocol is unavailable or organization policy blocks every route; choose an allowed route and refresh checks.".into(),
                ),
                Language::Indonesian => (
                    "Tidak ada jalur yang dapat digunakan".into(),
                    "Buka Diagnostik untuk melihat protokol yang tidak tersedia atau rute yang diblokir kebijakan organisasi; pilih jalur yang diizinkan lalu segarkan pemeriksaan.".into(),
                ),
                Language::German => (
                    "Kein nutzbarer Weg erkannt".into(),
                    "In der Diagnose prüfen, ob ein Protokoll fehlt oder die Organisationsrichtlinie alle Wege blockiert; einen erlaubten Weg wählen und erneut prüfen.".into(),
                ),
            };
        };
        let label = match self.lang {
            Language::English if route.blocked_by_policy => {
                "Blocked by organization policy".to_owned()
            }
            Language::Indonesian if route.blocked_by_policy => {
                "Diblokir kebijakan organisasi".to_owned()
            }
            Language::German if route.blocked_by_policy => {
                "Durch Organisationsrichtlinie blockiert".to_owned()
            }
            Language::English if route.recommended => "Recommended integration".to_owned(),
            Language::English => "Route capability check".to_owned(),
            Language::Indonesian if route.recommended => {
                "Integrasi yang direkomendasikan".to_owned()
            }
            Language::Indonesian => "Pemeriksaan kemampuan jalur".to_owned(),
            Language::German if route.recommended => "Empfohlene Integration".to_owned(),
            Language::German => "Prüfung der Routenkapazität".to_owned(),
        };
        let policy_detail = if route.blocked_by_policy {
            match self.lang {
                Language::English => "Organization policy blocks this route; ask the administrator to allow a supported route or choose an allowed one.",
                Language::Indonesian => "Kebijakan organisasi memblokir jalur ini; minta administrator mengizinkan jalur yang didukung atau pilih jalur yang diizinkan.",
                Language::German => "Die Organisationsrichtlinie blockiert diesen Weg; die Administration um Freigabe eines unterstützten Wegs bitten oder einen erlaubten Weg wählen.",
            }
        } else {
            ""
        };
        let detail = format!(
            "{}{}{} · {} · {} · focus tracking {}",
            policy_detail,
            if policy_detail.is_empty() { "" } else { " · " },
            self.onboarding_route_component_detail(true, route.capture_label, route.capture_state,),
            self.onboarding_route_component_detail(
                false,
                route.injection_label,
                route.injection_state,
            ),
            self.route_trust_status(RouteTrustDetails {
                route: route.capture_label,
                maturity: if route.certified {
                    "certified"
                } else {
                    "experimental"
                },
                sensitive_fields: Some(route.sensitive_fields),
                trigger_precondition: route.trigger_precondition,
                output_transaction: route.output_transaction,
                application_result: route.application_result,
                certified: route.certified,
            },),
            if route.focus_tracking {
                match self.lang {
                    Language::English => "available",
                    Language::Indonesian => "tersedia",
                    Language::German => "verfügbar",
                }
            } else {
                match self.lang {
                    Language::English => "unavailable",
                    Language::Indonesian => "tidak tersedia",
                    Language::German => "nicht verfügbar",
                }
            }
        );
        (label, detail)
    }

    /// Describe capture and insertion independently, including a remedy for
    /// each state. `Implemented` is deliberately worded as unknown rather than
    /// as either unsupported or requiring permission.
    pub fn onboarding_route_component_detail(
        &self,
        capture: bool,
        backend: &str,
        state: BackendState,
    ) -> String {
        use BackendState as S;
        let (stage, state_label, action) = match self.lang {
            Language::English => (
                if capture { "Capture" } else { "Insertion" },
                match state {
                    S::Available => "available",
                    S::RequiresPermission => "permission required",
                    S::ActionRequired => "action needed",
                    S::Implemented => "not yet checked",
                    S::Unavailable => "unsupported in this session",
                    S::NotImplemented => "not implemented",
                },
                match (capture, state) {
                    (_, S::Available) => "test this stage in a normal text field",
                    (true, S::ActionRequired) if backend == "ibus" => {
                        "review the IBus registration, shared-component security, or desktop-routing finding in Diagnostics → IBus, then refresh"
                    }
                    (true, S::ActionRequired) => "finish guided route setup and refresh this check",
                    (false, S::ActionRequired) => match backend {
                        "ibus" => "complete guided IBus setup and explicitly review non-atomic replacement consent",
                        "input-method-v2" => "complete the compositor's input-method setup, then recheck insertion support",
                        _ => "complete the required setup step and refresh this check",
                    },
                    (true, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "complete guided IBus registration and source selection; this is not raw-device access",
                            "input-method-v2" => "complete the compositor's guided input-method setup, then recheck support",
                            _ => "check input-device access and follow the guided permission steps",
                        }
                    }
                    (false, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "review and explicitly allow compatibility replacement in Engine settings",
                            "input-method-v2" => "complete the compositor's input-method setup, then recheck insertion support",
                            "libei" => "request desktop portal access; portal support is not confirmed yet",
                            _ => "complete the required output authorization, then recheck insertion",
                        }
                    }
                    (_, S::Implemented) if !capture && backend == "libei" => {
                        "run `wayexpand backend test --injection` to check EIS support; it asks before inserting a harmless marker"
                    }
                    (_, S::Implemented) => "refresh diagnostics to check support",
                    (_, S::Unavailable) => match backend {
                        "ibus" => "this IBus route is unsupported in this session; open Diagnostics → IBus to see which capability is missing, then choose an available route",
                        "input-method-v2" => "this compositor lacks input-method-v2; choose another integration in Setup",
                        "libei" => "no usable EIS injection route was detected; choose another output route",
                        "wlroots-virtual-keyboard" => "this compositor lacks wlroots virtual-keyboard; choose another output route",
                        _ => "choose another integration in Setup, then refresh",
                    },
                    (_, S::NotImplemented) => "choose a different integration in Setup",
                },
            ),
            Language::Indonesian => (
                if capture { "Tangkapan" } else { "Penyisipan" },
                match state {
                    S::Available => "tersedia",
                    S::RequiresPermission => "perlu izin",
                    S::ActionRequired => "perlu tindakan",
                    S::Implemented => "belum diperiksa",
                    S::Unavailable => "tidak didukung di sesi ini",
                    S::NotImplemented => "belum diimplementasikan",
                },
                match (capture, state) {
                    (_, S::Available) => "uji tahap ini di kolom teks biasa",
                    (true, S::ActionRequired) if backend == "ibus" => {
                        "tinjau temuan pendaftaran IBus, keamanan komponen bersama, atau perutean desktop di Diagnostik → IBus, lalu segarkan"
                    }
                    (true, S::ActionRequired) => "selesaikan penyiapan rute terpandu lalu segarkan pemeriksaan ini",
                    (false, S::ActionRequired) => match backend {
                        "ibus" => "selesaikan penyiapan IBus terpandu dan tinjau persetujuan penggantian non-atomik secara eksplisit",
                        "input-method-v2" => "selesaikan penyiapan input-method compositor, lalu periksa dukungan penyisipan kembali",
                        _ => "selesaikan langkah penyiapan yang diperlukan lalu segarkan pemeriksaan ini",
                    },
                    (true, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "selesaikan pendaftaran IBus dan pemilihan sumber terpandu; ini bukan akses perangkat mentah",
                            "input-method-v2" => "selesaikan penyiapan input-method terpandu dari compositor, lalu periksa dukungan kembali",
                            _ => "periksa akses perangkat input dan ikuti langkah izin terpandu",
                        }
                    }
                    (false, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "tinjau dan izinkan penggantian kompatibilitas secara eksplisit di Pengaturan Engine",
                            "input-method-v2" => "selesaikan penyiapan input-method compositor, lalu periksa dukungan penyisipan kembali",
                            "libei" => "minta akses portal desktop; dukungan portal belum dipastikan",
                            _ => "selesaikan otorisasi keluaran yang diperlukan, lalu periksa penyisipan kembali",
                        }
                    }
                    (_, S::Implemented) if !capture && backend == "libei" => {
                        "jalankan `wayexpand backend test --injection` untuk memeriksa dukungan EIS; perintah meminta persetujuan sebelum menyisipkan penanda aman"
                    }
                    (_, S::Implemented) => "segarkan diagnostik untuk memeriksa dukungan",
                    (_, S::Unavailable) => match backend {
                        "ibus" => "rute IBus ini tidak didukung di sesi ini; buka Diagnostik → IBus untuk melihat kemampuan yang tidak tersedia, lalu pilih rute lain",
                        "input-method-v2" => "compositor ini tidak menyediakan input-method-v2; pilih integrasi lain",
                        "libei" => "rute injeksi EIS yang dapat digunakan tidak terdeteksi; pilih keluaran lain",
                        "wlroots-virtual-keyboard" => "compositor ini tidak menyediakan wlroots virtual-keyboard; pilih keluaran lain",
                        _ => "pilih integrasi lain di Penyiapan, lalu segarkan",
                    },
                    (_, S::NotImplemented) => "pilih integrasi lain di Penyiapan",
                },
            ),
            Language::German => (
                if capture { "Erfassung" } else { "Einfügen" },
                match state {
                    S::Available => "verfügbar",
                    S::RequiresPermission => "Berechtigung nötig",
                    S::ActionRequired => "Aktion erforderlich",
                    S::Implemented => "noch nicht geprüft",
                    S::Unavailable => "in dieser Sitzung nicht unterstützt",
                    S::NotImplemented => "nicht implementiert",
                },
                match (capture, state) {
                    (_, S::Available) => "diesen Schritt in einem normalen Textfeld testen",
                    (true, S::ActionRequired) if backend == "ibus" => {
                        "den Hinweis zu IBus-Registrierung, Sicherheit der gemeinsamen Komponente oder Desktop-Route unter Diagnose → IBus prüfen und danach erneut aktualisieren"
                    }
                    (true, S::ActionRequired) => "die geführte Routeneinrichtung abschließen und diese Prüfung aktualisieren",
                    (false, S::ActionRequired) => match backend {
                        "ibus" => "die geführte IBus-Einrichtung abschließen und die Zustimmung zur nicht-atomaren Ersetzung ausdrücklich prüfen",
                        "input-method-v2" => "die Eingabemethode des Compositors einrichten und die Einfügeunterstützung erneut prüfen",
                        _ => "den erforderlichen Einrichtungsschritt abschließen und diese Prüfung aktualisieren",
                    },
                    (true, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "die geführte IBus-Registrierung und Quellenauswahl abschließen; dies ist kein Rohgerätezugriff",
                            "input-method-v2" => "die geführte Eingabemethode des Compositors einrichten und die Unterstützung erneut prüfen",
                            _ => "Eingabegerätezugriff prüfen und den geführten Berechtigungsschritten folgen",
                        }
                    }
                    (false, S::RequiresPermission) => {
                        match backend {
                            "ibus" => "die Kompatibilitätsersetzung in den Engine-Einstellungen prüfen und ausdrücklich erlauben",
                            "input-method-v2" => "die Eingabemethode des Compositors einrichten und die Einfügeunterstützung erneut prüfen",
                            "libei" => "Desktop-Portalzugriff anfordern; Portalunterstützung ist noch unbestätigt",
                            _ => "die erforderliche Ausgabeautorisierung abschließen und das Einfügen erneut prüfen",
                        }
                    }
                    (_, S::Implemented) if !capture && backend == "libei" => {
                        "`wayexpand backend test --injection` ausführen, um EIS zu prüfen; vor dem Einfügen einer harmlosen Markierung wird gefragt"
                    }
                    (_, S::Implemented) => "Diagnose aktualisieren, um die Unterstützung zu prüfen",
                    (_, S::Unavailable) => match backend {
                        "ibus" => "Dieser IBus-Weg wird in dieser Sitzung nicht unterstützt. Öffnen Sie Diagnose → IBus, um die fehlende Funktion zu sehen, und wählen Sie anschließend einen verfügbaren Weg.",
                        "input-method-v2" => "dieser Compositor bietet input-method-v2 nicht an; eine andere Integration wählen",
                        "libei" => "kein nutzbarer EIS-Ausgabeweg erkannt; einen anderen Ausgabeweg wählen",
                        "wlroots-virtual-keyboard" => "dieser Compositor bietet wlroots virtual-keyboard nicht an; einen anderen Ausgabeweg wählen",
                        _ => "in der Einrichtung eine andere Integration wählen und erneut prüfen",
                    },
                    (_, S::NotImplemented) => "in der Einrichtung eine andere Integration wählen",
                },
            ),
        };
        format!("{stage} ({backend}): {state_label}; {action}.")
    }

    pub fn onboarding_evdev_setup(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Enable raw keyboard capture — no password-field detection (advanced)…"
            }
            Language::Indonesian => "Aktifkan tangkapan papan ketik mentah — lanjutan…",
            Language::German => "Rohe Tastaturerfassung aktivieren — erweitert…",
        }
    }
}

#[cfg(test)]
mod route_trust_tests {
    use super::*;
    use crate::lang::Language;

    #[test]
    fn recommended_route_does_not_turn_unsupported_or_unknown_into_permission_errors() {
        let strings = Strings::new(Language::English);
        let unsupported = strings.onboarding_route_component_detail(
            true,
            "input-method-v2",
            crate::BackendState::Unavailable,
        );
        assert!(
            unsupported.contains("unsupported in this session"),
            "{unsupported}"
        );
        assert!(
            unsupported.contains("choose another integration"),
            "{unsupported}"
        );
        assert!(!unsupported.contains("permission"), "{unsupported}");

        let unknown = strings.onboarding_route_component_detail(
            false,
            "libei",
            crate::BackendState::Implemented,
        );
        assert!(unknown.contains("not yet checked"), "{unknown}");
        assert!(
            unknown.contains("wayexpand backend test --injection"),
            "unknown injection support should point to the capability test: {unknown}"
        );
        assert!(!unknown.contains("permission"), "{unknown}");

        let authorization = strings.onboarding_route_component_detail(
            false,
            "libei",
            crate::BackendState::RequiresPermission,
        );
        assert!(
            authorization.contains("request desktop portal access"),
            "{authorization}"
        );
        assert!(
            authorization.contains("not confirmed yet"),
            "{authorization}"
        );

        let ibus_consent = strings.onboarding_route_component_detail(
            false,
            "ibus",
            crate::BackendState::ActionRequired,
        );
        assert!(
            ibus_consent.contains("explicitly review non-atomic replacement consent"),
            "{ibus_consent}"
        );
        assert!(!ibus_consent.contains("portal"), "{ibus_consent}");

        let unavailable_ibus = strings.onboarding_route_component_detail(
            false,
            "ibus",
            crate::BackendState::Unavailable,
        );
        assert!(unavailable_ibus.contains("unsupported in this session"));
        assert!(unavailable_ibus.contains("Diagnostics → IBus"));
        assert!(
            !unavailable_ibus.contains("permission"),
            "{unavailable_ibus}"
        );
    }

    fn summary(
        language: Language,
        route: &str,
        maturity: &str,
        sensitive_fields: Option<bool>,
        trigger_precondition: &str,
        output_transaction: &str,
        certified: bool,
    ) -> String {
        Strings::new(language).route_trust_status(RouteTrustDetails {
            route,
            maturity,
            sensitive_fields,
            trigger_precondition,
            output_transaction,
            application_result: "unobservable",
            certified,
        })
    }

    #[test]
    fn route_summary_separates_precondition_transaction_and_application_result() {
        let unverified = summary(
            Language::English,
            "Evdev + libei",
            "experimental",
            Some(false),
            "none",
            "separate_delete_and_key_events",
            false,
        );
        assert!(
            unverified.contains("trigger target not verified"),
            "{unverified}"
        );
        assert!(
            unverified.contains("non-atomic delete and key events"),
            "{unverified}"
        );
        assert!(
            unverified.contains("application result unverified"),
            "{unverified}"
        );
        assert!(
            unverified.contains("no sensitive-field protection"),
            "{unverified}"
        );
        assert!(!unverified.contains("best effort"), "{unverified}");

        let verified = summary(
            Language::English,
            "IBus",
            "experimental",
            Some(true),
            "surrounding_text_matches_trigger",
            "separate_delete_and_commit",
            false,
        );
        assert!(
            verified.contains("trigger checked against surrounding text"),
            "{verified}"
        );
        assert!(
            verified.contains("application result unverified"),
            "{verified}"
        );
        assert!(
            verified.contains("sensitive-field signal available"),
            "{verified}"
        );
        assert!(
            verified.contains("non-atomic delete and commit"),
            "{verified}"
        );
        assert!(!verified.contains("atomic protocol batch"), "{verified}");
    }

    #[test]
    fn atomic_route_never_claims_application_result_was_verified() {
        for (language, transaction, app_unverified, misleading) in [
            (
                Language::English,
                "atomic protocol batch",
                "application result unverified",
                "application result verified",
            ),
            (
                Language::Indonesian,
                "batch protokol atomik",
                "hasil aplikasi belum terverifikasi",
                "hasil aplikasi terverifikasi",
            ),
            (
                Language::German,
                "atomarer Protokollbatch",
                "Anwendungsergebnis unverifiziert",
                "Anwendungsergebnis verifiziert",
            ),
        ] {
            let status = summary(
                language,
                "route",
                "experimental",
                None,
                "surrounding_text_matches_trigger",
                "atomic_protocol_batch",
                false,
            );
            assert!(status.contains(transaction), "{language:?}: {status}");
            assert!(status.contains(app_unverified), "{language:?}: {status}");
            assert!(!status.contains(misleading), "{language:?}: {status}");
        }
    }

    #[test]
    fn missing_live_status_is_reported_as_unknown_sensitive_protection() {
        let text = summary(
            Language::English,
            "input-method-v2",
            "supported",
            None,
            "surrounding_text_matches_trigger",
            "atomic_protocol_batch",
            false,
        );
        assert!(
            text.contains("sensitive-field protection unknown"),
            "{text}"
        );
    }
}
