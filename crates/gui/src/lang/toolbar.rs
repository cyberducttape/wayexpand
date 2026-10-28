use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn title(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet library",
            Language::Indonesian => "Pustaka cuplikan",
            Language::German => "Snippet-Bibliothek",
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

    pub fn unsaved_changes(&self) -> &'static str {
        match self.lang {
            Language::English => "● Unsaved changes",
            Language::Indonesian => "● Perubahan belum disimpan",
            Language::German => "● Ungespeicherte Änderungen",
        }
    }

    pub fn settings(&self) -> &'static str {
        match self.lang {
            Language::English => "Settings",
            Language::Indonesian => "Pengaturan",
            Language::German => "Einstellungen",
        }
    }

    pub fn more_actions(&self) -> &'static str {
        match self.lang {
            Language::English => "More actions",
            Language::Indonesian => "Tindakan lainnya",
            Language::German => "Weitere Aktionen",
        }
    }

    pub fn sync_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Sync library",
            Language::Indonesian => "Sinkronkan pustaka",
            Language::German => "Bibliothek synchronisieren",
        }
    }

    pub fn sync_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Synchronizing the library with Git…",
            Language::Indonesian => "Menyinkronkan pustaka dengan Git…",
            Language::German => "Bibliothek wird mit Git synchronisiert…",
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

    pub fn import_espanso(&self) -> &'static str {
        match self.lang {
            Language::English => "Import Espanso",
            Language::Indonesian => "Impor Espanso",
            Language::German => "Espanso importieren",
        }
    }

    pub fn diagnostics(&self) -> &'static str {
        match self.lang {
            Language::English => "Diagnostics",
            Language::Indonesian => "Diagnostik",
            Language::German => "Diagnose",
        }
    }

    pub fn pause(&self) -> &'static str {
        match self.lang {
            Language::English => "Pause",
            Language::Indonesian => "Jeda",
            Language::German => "Pause",
        }
    }

    pub fn resume(&self) -> &'static str {
        match self.lang {
            Language::English => "Resume",
            Language::Indonesian => "Lanjutkan",
            Language::German => "Fortsetzen",
        }
    }

    pub fn running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "● Running",
            Language::Indonesian => "● Berjalan",
            Language::German => "● Aktiv",
        }
    }

    pub fn daemon_running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Running",
            Language::Indonesian => "Daemon: Berjalan",
            Language::German => "Daemon: Aktiv",
        }
    }

    pub fn daemon_restart_required_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Restart daemon",
            Language::Indonesian => "Mulai ulang daemon",
            Language::German => "Daemon neu starten",
        }
    }

    pub fn daemon_restart_required_help(&self) -> &'static str {
        match self.lang {
            Language::English => "The running daemon is from a different build. Restart the WayExpand user service before relying on this editor's fixes.",
            Language::Indonesian => "Daemon yang berjalan berasal dari build berbeda. Mulai ulang layanan pengguna WayExpand sebelum mengandalkan perbaikan di editor ini.",
            Language::German => "Der laufende Daemon stammt aus einem anderen Build. Starten Sie den WayExpand-Benutzerdienst neu, bevor Sie sich auf Korrekturen dieses Editors verlassen.",
        }
    }

    pub fn daemon_build_unverified_help(&self) -> &'static str {
        match self.lang {
            Language::English => "The running daemon's build identity is missing or unknown. Runtime safety guarantees cannot be matched to this GUI build.",
            Language::Indonesian => "Identitas build daemon yang berjalan tidak tersedia atau tidak diketahui. Jaminan keamanan runtime tidak dapat dicocokkan dengan build GUI ini.",
            Language::German => "Die Build-Identität des laufenden Daemons fehlt oder ist unbekannt. Laufzeit-Sicherheitsgarantien lassen sich diesem GUI-Build nicht zuordnen.",
        }
    }

    pub fn daemon_unreachable_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Unreachable",
            Language::Indonesian => "Daemon: Tidak dapat dijangkau",
            Language::German => "Daemon: Nicht erreichbar",
        }
    }

    pub fn daemon_not_enabled_status(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand: Not enabled",
            Language::Indonesian => "WayExpand: Belum diaktifkan",
            Language::German => "WayExpand: Nicht aktiviert",
        }
    }

    pub fn daemon_unknown_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon: Checking…",
            Language::Indonesian => "Daemon: Memeriksa…",
            Language::German => "Daemon: Wird geprüft…",
        }
    }

    pub fn route_trust_status(
        &self,
        route: &str,
        maturity: &str,
        sensitive_fields: bool,
        atomic_replace: bool,
        replacement_assurance: &str,
        certified: bool,
    ) -> String {
        let semantics = match (self.lang, sensitive_fields, atomic_replace) {
            (Language::English, true, true) => "sensitive-field aware · atomic replacement",
            (Language::Indonesian, true, true) => "sadar kolom sensitif · penggantian atomik",
            (Language::English, true, false) => "sensitive-field aware · non-atomic replacement",
            (Language::Indonesian, true, false) => "sadar kolom sensitif · penggantian non-atomik",
            (Language::English, false, true) => "global keyboard visibility · atomic replacement",
            (Language::Indonesian, false, true) => {
                "visibilitas papan ketik global · penggantian atomik"
            }
            (Language::English, false, false) => {
                "global keyboard visibility · non-atomic replacement"
            }
            (Language::Indonesian, false, false) => {
                "visibilitas papan ketik global · penggantian non-atomik"
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
            (Language::Indonesian, true) => "tersertifikasi",
            (Language::English, false) => "certification pending",
            (Language::Indonesian, false) => "sertifikasi tertunda",
            (Language::German, true) => "zertifiziert",
            (Language::German, false) => "Zertifizierung ausstehend",
        };
        let assurance = match (self.lang, replacement_assurance) {
            (Language::English, "verified") => "target text verified",
            (Language::English, "focus-tracked-best-effort") => "focus-tracked best effort",
            (Language::English, "unverified") => "target text unverified",
            (Language::English, "blocked") => "replacement blocked",
            (Language::English, "unavailable") => "replacement unavailable",
            (Language::English, _) => "replacement assurance unknown",
            (Language::Indonesian, "verified") => "teks tujuan terverifikasi",
            (Language::Indonesian, "focus-tracked-best-effort") => {
                "upaya terbaik dengan pelacakan fokus"
            }
            (Language::Indonesian, "unverified") => "teks tujuan tidak terverifikasi",
            (Language::Indonesian, "blocked") => "penggantian diblokir",
            (Language::Indonesian, "unavailable") => "penggantian tidak tersedia",
            (Language::Indonesian, _) => "jaminan penggantian tidak diketahui",
            (Language::German, "verified") => "verifizierter Zieltext",
            (Language::German, "focus-tracked-best-effort") => "fokusgeprüft nach bestem Bemühen",
            (Language::German, "unverified") => "unverifizierter Zieltext",
            (Language::German, "blocked") => "Ersetzung blockiert",
            (Language::German, "unavailable") => "Ersetzung nicht verfügbar",
            (Language::German, _) => "Ersetzungssicherheit unbekannt",
        };
        format!("{route} · {assurance} · {maturity} · {semantics} · {evidence}")
    }

    pub fn route_connected_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Ready",
            Language::Indonesian => "Integrasi pengetikan: Siap",
            Language::German => "Tastaturintegration: Bereit",
        }
    }

    /// Connected with full protection, but the route is not certified, so
    /// expansion has not been verified on this desktop.
    pub fn route_running_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Running",
            Language::Indonesian => "Integrasi pengetikan: Berjalan",
            Language::German => "Tastaturintegration: Aktiv",
        }
    }
    pub fn route_limited_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Limited protection",
            Language::Indonesian => "Integrasi pengetikan: Perlindungan terbatas",
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
            Language::Indonesian => "Integrasi pengetikan: Perlu izin",
            Language::German => "Tastaturintegration: Berechtigung erforderlich",
        }
    }

    pub fn route_portal_revoked_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Permission revoked",
            Language::Indonesian => "Integrasi pengetikan: Izin dicabut",
            Language::German => "Tastaturintegration: Berechtigung widerrufen",
        }
    }

    pub fn route_unsupported_status(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing integration: Unsupported",
            Language::Indonesian => "Integrasi pengetikan: Tidak didukung",
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
            Language::Indonesian => "Detail teknis",
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

    pub fn reload(&self) -> &'static str {
        match self.lang {
            Language::English => "Reload",
            Language::Indonesian => "Muat ulang",
            Language::German => "Neu laden",
        }
    }

    pub fn search_placeholder(&self) -> &'static str {
        match self.lang {
            Language::English => "Search snippets…",
            Language::Indonesian => "Cari cuplikan…",
            Language::German => "Snippets durchsuchen…",
        }
    }

    pub fn search_fields(&self) -> &'static str {
        match self.lang {
            Language::English => "Search fields",
            Language::Indonesian => "Bidang pencarian",
            Language::German => "Suchfelder",
        }
    }

    pub fn search_triggers(&self) -> &'static str {
        match self.lang {
            Language::English => "Triggers",
            Language::Indonesian => "Pemicu",
            Language::German => "Kürzel",
        }
    }

    pub fn search_descriptions(&self) -> &'static str {
        match self.lang {
            Language::English => "Names and descriptions",
            Language::Indonesian => "Nama dan deskripsi",
            Language::German => "Namen und Beschreibungen",
        }
    }

    pub fn search_tags(&self) -> &'static str {
        match self.lang {
            Language::English => "Tags",
            Language::Indonesian => "Tag",
            Language::German => "Schlagwörter",
        }
    }

    pub fn search_replacements(&self) -> &'static str {
        match self.lang {
            Language::English => "Replacement content",
            Language::Indonesian => "Isi teks pengganti",
            Language::German => "Eingefügter Text",
        }
    }

    pub fn welcome_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Welcome to WayExpand",
            Language::Indonesian => "Selamat datang di WayExpand",
            Language::German => "Willkommen bei WayExpand",
        }
    }

    pub fn welcome_intro(&self) -> &'static str {
        match self.lang {
            Language::English => "Text expansion for Wayland, in three steps.",
            Language::Indonesian => "Ekspansi teks untuk Wayland, dalam tiga langkah.",
            Language::German => "Textexpansion für Wayland, in drei Schritten.",
        }
    }

    pub fn select_snippet_prompt(&self) -> &'static str {
        match self.lang {
            Language::English => "Select a snippet from your library to edit it.",
            Language::Indonesian => "Pilih cuplikan dari pustaka Anda untuk menyuntingnya.",
            Language::German => "Wählen Sie ein Snippet aus Ihrer Bibliothek zum Bearbeiten aus.",
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

    pub fn onboarding_probe_caveat(&self) -> &'static str {
        match self.lang {
            Language::English => "Backend probes show what may be available; they do not certify reliable input or output.",
            Language::Indonesian => "Pemeriksaan backend menunjukkan apa yang mungkin tersedia; pemeriksaan ini tidak menjamin masukan atau keluaran yang andal.",
            Language::German => "Backend-Prüfungen zeigen mögliche Verfügbarkeit, zertifizieren aber keine zuverlässige Ein- oder Ausgabe.",
        }
    }

    pub fn onboarding_app_caveat(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Application filters depend on desktop integration and may be unavailable."
            }
            Language::Indonesian => {
                "Filter aplikasi bergantung pada integrasi desktop dan mungkin tidak tersedia."
            }
            Language::German => {
                "App-Filter benötigen Desktop-Integration und sind möglicherweise nicht verfügbar."
            }
        }
    }

    pub fn create_test_snippet(&self) -> &'static str {
        match self.lang {
            Language::English => "Create test snippet",
            Language::Indonesian => "Buat cuplikan uji",
            Language::German => "Testsnippet erstellen",
        }
    }

    pub fn onboarding_try_text(&self) -> &'static str {
        match self.lang {
            Language::English => "Type :wayexpand-test in the Matcher preview box, then use the separate desktop setup checks before trying it in another app. Never test in a password field.",
            Language::Indonesian => "Ketik :wayexpand-test di kotak Pratinjau pencocok, lalu gunakan pemeriksaan penyiapan desktop terpisah sebelum mencobanya di aplikasi lain. Jangan pernah menguji di kolom kata sandi.",
            Language::German => "Tippen Sie :wayexpand-test in der Matcher-Vorschau ein und verwenden Sie danach die getrennten Desktop-Tests, bevor Sie es in einer anderen App probieren. Nie in einem Passwortfeld testen.",
        }
    }

    pub fn onboarding_sample_description(&self) -> &'static str {
        match self.lang {
            Language::English => "A test expansion for your first-run check",
            Language::Indonesian => "Ekspansi uji untuk pemeriksaan pertama Anda",
            Language::German => "Eine Textersetzung zum Ausprobieren",
        }
    }

    pub fn onboarding_sample_category(&self) -> &'static str {
        match self.lang {
            Language::English => "Getting started",
            Language::Indonesian => "Memulai",
            Language::German => "Erste Schritte",
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

    pub fn onboarding_keyboard_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Keyboard capture",
            Language::Indonesian => "Tangkapan papan ketik",
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
                Language::Indonesian => (
                    "Belum terdeteksi rute aman yang lengkap".into(),
                    "Segarkan diagnostik setelah sesi desktop dimulai".into(),
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
                Language::Indonesian => "Terdeteksi di mesin ini",
                Language::German => "Auf diesem Rechner erkannt",
            }
        } else {
            match self.lang {
                Language::English => "Permission required before setup",
                Language::Indonesian => "Perlu izin sebelum penyiapan",
                Language::German => "Vor der Einrichtung ist eine Berechtigung nötig",
            }
        };
        let label = match self.lang {
            Language::English => "Safe integration".to_owned(),
            Language::Indonesian => "Integrasi aman".to_owned(),
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
                } else {
                    properties
                        .push("best-effort replacement; strict mode blocks it by default".into());
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
            Language::Indonesian => {
                let mut properties = vec![state.to_owned()];
                if route.sensitive_fields {
                    properties.push(
                        "perlindungan terbaik untuk konteks yang tidak didukung atau sensitif"
                            .into(),
                    );
                }
                if route.atomic_replace {
                    properties.push("penggantian atomik".into());
                } else {
                    properties.push(
                        "penggantian upaya terbaik; mode ketat memblokir secara default".into(),
                    );
                }
                if route.focus_tracking {
                    properties.push("pelacakan fokus aplikasi".into());
                }
                properties.push(if route.certified {
                    "tersertifikasi".into()
                } else {
                    "eksperimental hingga tersertifikasi".into()
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
                } else {
                    properties.push(
                        "Ersetzung nach bestem Bemühen; strikter Modus blockiert standardmäßig"
                            .into(),
                    );
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
            Language::Indonesian => "Integrasi yang direkomendasikan",
            Language::German => "Empfohlene Integration",
        }
    }

    pub fn onboarding_injection_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Text injection",
            Language::Indonesian => "Injeksi teks",
            Language::German => "Texteingabe",
        }
    }

    pub fn onboarding_capture_detail(&self, backend: &str) -> String {
        match self.lang {
            Language::English => format!("Capture: {backend}"),
            Language::Indonesian => format!("Tangkapan: {backend}"),
            Language::German => format!("Eingabe: {backend}"),
        }
    }

    pub fn onboarding_injection_detail(&self, backend: &str) -> String {
        match self.lang {
            Language::English => format!("Output: {backend}"),
            Language::Indonesian => format!("Keluaran: {backend}"),
            Language::German => format!("Ausgabe: {backend}"),
        }
    }

    pub fn onboarding_backend_state(&self, state: wayexpand_core::BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English, wayexpand_core::BackendState::Available) => "Detected",
            (Language::Indonesian, wayexpand_core::BackendState::Available) => "Terdeteksi",
            (Language::German, wayexpand_core::BackendState::Available) => "Erkannt",
            (Language::English, wayexpand_core::BackendState::RequiresPermission) => {
                "Permission required"
            }
            (Language::Indonesian, wayexpand_core::BackendState::RequiresPermission) => {
                "Perlu izin"
            }
            (Language::German, wayexpand_core::BackendState::RequiresPermission) => {
                "Berechtigung erforderlich"
            }
            (Language::English, wayexpand_core::BackendState::Implemented) => {
                "Supported for testing · not verified"
            }
            (Language::Indonesian, wayexpand_core::BackendState::Implemented) => {
                "Didukung untuk pengujian · belum diverifikasi"
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
                Language::Indonesian,
                wayexpand_core::BackendState::Unavailable
                | wayexpand_core::BackendState::NotImplemented,
            ) => "Tidak tersedia",
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
            Language::Indonesian => "Coba",
            Language::German => "Ausprobieren",
        }
    }

    pub fn onboarding_evdev_setup(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable raw keyboard capture — advanced…",
            Language::Indonesian => "Aktifkan tangkapan papan ketik mentah — lanjutan…",
            Language::German => "Rohe Tastaturerfassung aktivieren — erweitert…",
        }
    }

    pub fn evdev_setup_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable raw keyboard capture — advanced",
            Language::Indonesian => "Aktifkan tangkapan papan ketik mentah — lanjutan",
            Language::German => "Rohe Tastaturerfassung aktivieren — erweitert",
        }
    }

    pub fn evdev_setup_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Works in more applications, but can observe password-field typing because this mode has no sensitive-field signal. This is an explicit opt-in. Device permissions are a separate system-wide change and are never made by this GUI.",
            Language::Indonesian => "Berfungsi di lebih banyak aplikasi, tetapi dapat mengamati pengetikan di kolom kata sandi karena mode ini tidak memiliki sinyal kolom sensitif. Ini adalah pilihan eksplisit. Izin perangkat merupakan perubahan terpisah di seluruh sistem dan tidak pernah dilakukan oleh GUI ini.",
            Language::German => "Funktioniert in mehr Anwendungen, kann aber Eingaben in Passwortfeldern beobachten, weil dieser Modus kein Signal für sensible Felder hat. Dies ist eine ausdrückliche Zustimmung. Geräteberechtigungen sind eine separate systemweite Änderung und werden von dieser GUI niemals vorgenommen.",
        }
    }

    pub fn evdev_setup_acknowledge(&self) -> &'static str {
        match self.lang {
            Language::English => "I understand the raw-input and password-field limitations",
            Language::Indonesian => "Saya memahami batasan masukan mentah dan kolom kata sandi",
            Language::German => "Ich verstehe die Einschränkungen bei Rohdaten und Passwortfeldern",
        }
    }

    pub fn evdev_setup_steps(&self) -> &'static str {
        match self.lang {
            Language::English => "If keyboard-device access is not already configured, first complete the administrator permission step in Getting Started. Then copy the command below into a terminal: it enables the user service, and the desktop may ask you to approve the input portal. Return here, refresh diagnostics, and test in a normal text field.",
            Language::Indonesian => "Jika akses perangkat papan ketik belum dikonfigurasi, selesaikan dahulu langkah izin administrator di Memulai. Lalu salin perintah di bawah ke terminal: perintah ini mengaktifkan layanan pengguna, dan desktop mungkin meminta Anda menyetujui portal masukan. Kembali ke sini, segarkan diagnostik, dan uji di kolom teks biasa.",
            Language::German => "Falls der Zugriff auf Tastaturgeräte noch nicht eingerichtet ist, schließen Sie zuerst den Administrator-Schritt in der Installationsanleitung ab. Kopieren Sie danach den folgenden Befehl in ein Terminal: Er aktiviert den Benutzerdienst; der Desktop kann um Freigabe des Eingabeportals bitten. Kehren Sie zurück, aktualisieren Sie die Diagnose und testen Sie in einem normalen Textfeld.",
        }
    }
}

#[cfg(test)]
mod route_trust_tests {
    use super::*;
    use crate::lang::Language;

    #[test]
    fn route_summary_distinguishes_unverified_from_best_effort_and_atomicity() {
        let strings = Strings::new(Language::English);
        let unverified = strings.route_trust_status(
            "Evdev + libei",
            "experimental",
            false,
            false,
            "unverified",
            false,
        );
        assert!(
            unverified.contains("target text unverified"),
            "{unverified}"
        );
        assert!(
            unverified.contains("non-atomic replacement"),
            "{unverified}"
        );
        assert!(!unverified.contains("best effort"), "{unverified}");

        let verified =
            strings.route_trust_status("IBus", "experimental", true, false, "verified", false);
        assert!(verified.contains("target text verified"), "{verified}");
        assert!(verified.contains("non-atomic replacement"), "{verified}");
        assert!(!verified.contains("· atomic replacement"), "{verified}");
    }
}
