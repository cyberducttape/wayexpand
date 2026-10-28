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
            Language::English => "From your first setup check to a real expansion in an app.",
            Language::Indonesian => "Dari pemeriksaan awal hingga ekspansi nyata di aplikasi.",
            Language::German => "Vom ersten Einrichtungscheck bis zur Ersetzung in einer App.",
        }
    }

    pub fn onboarding_install_summary(&self, version: &str) -> String {
        match self.lang {
            Language::English => format!("WayExpand GUI {version} is running. The next step starts its user service."),
            Language::Indonesian => format!("WayExpand GUI {version} sedang berjalan. Langkah berikutnya menjalankan layanan pengguna."),
            Language::German => format!("WayExpand GUI {version} läuft. Im nächsten Schritt wird der Benutzerdienst gestartet."),
        }
    }

    pub fn onboarding_backend_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Verify this desktop session",
            Language::Indonesian => "Periksa sesi desktop ini",
            Language::German => "Diese Desktop-Sitzung prüfen",
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

    pub fn onboarding_unverified_daemon_state(&self) -> &'static str {
        match self.lang {
            Language::English => "Not ready: the daemon's build identity is unknown. Install matching builds with a recorded commit identity before testing snippets.",
            Language::Indonesian => "Belum siap: identitas build daemon tidak diketahui. Pasang build yang cocok dengan identitas commit tercatat sebelum menguji cuplikan.",
            Language::German => "Noch nicht bereit: Die Build-Identität des Daemons ist unbekannt. Installieren Sie passende Builds mit aufgezeichneter Commit-Identität, bevor Sie Snippets testen.",
        }
    }

    pub fn onboarding_refresh_checks(&self) -> &'static str {
        match self.lang {
            Language::English => "Refresh session checks",
            Language::Indonesian => "Segarkan pemeriksaan sesi",
            Language::German => "Sitzungsprüfung aktualisieren",
        }
    }

    pub fn onboarding_setup_recovery(&self) -> &'static str {
        match self.lang {
            Language::English => "If setup cannot find `wayexpand`, install WayExpand with your package or build instructions, then reopen the GUI. The setup command is shown in the status message and is never run as root.",
            Language::Indonesian => "Jika perintah `wayexpand` tidak ditemukan, pasang WayExpand melalui paket atau petunjuk build, lalu buka kembali GUI. Perintah penyiapan terlihat pada status dan tidak pernah dijalankan sebagai root.",
            Language::German => "Wird `wayexpand` nicht gefunden, installieren Sie WayExpand anhand der Paket- oder Build-Anleitung und öffnen Sie die GUI erneut. Der Setup-Befehl steht in der Statusmeldung und läuft nie als root.",
        }
    }

    pub fn onboarding_sample_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Test snippet saved and ready to preview.",
            Language::Indonesian => "Cuplikan uji tersimpan dan siap dipratinjau.",
            Language::German => "Test-Snippet gespeichert und bereit für die Vorschau.",
        }
    }

    pub fn onboarding_create_sample_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Create and save the test snippet first.",
            Language::Indonesian => "Buat dan simpan cuplikan uji terlebih dahulu.",
            Language::German => "Erstellen und speichern Sie zuerst das Test-Snippet.",
        }
    }

    pub fn onboarding_manual_check_note(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand cannot inspect another app’s text field. Your confirmation is a manual result, not an automatic safety or compatibility certification.",
            Language::Indonesian => "WayExpand tidak dapat memeriksa kolom teks aplikasi lain. Konfirmasi Anda adalah hasil manual, bukan sertifikasi keamanan atau kompatibilitas otomatis.",
            Language::German => "WayExpand kann das Textfeld einer anderen App nicht prüfen. Ihre Bestätigung ist ein manuelles Ergebnis, keine automatische Sicherheits- oder Kompatibilitätszertifizierung.",
        }
    }

    pub fn onboarding_open_diagnostics(&self) -> &'static str {
        match self.lang {
            Language::English => "Open diagnostics",
            Language::Indonesian => "Buka diagnostik",
            Language::German => "Diagnose öffnen",
        }
    }

    pub fn onboarding_matcher_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Check the Matcher preview",
            Language::Indonesian => "Periksa pratinjau pencocok",
            Language::German => "Matcher-Vorschau prüfen",
        }
    }

    pub fn onboarding_matcher_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Use the Matcher preview at the bottom of this window. Type the trigger exactly; it should become the expected text below. This checks your saved snippet and matcher locally, not keyboard capture or insertion.",
            Language::Indonesian => "Gunakan Pratinjau pencocok di bagian bawah jendela ini. Ketik pemicu persis; hasilnya harus menjadi teks yang diharapkan. Ini memeriksa cuplikan dan pencocok secara lokal, bukan tangkapan atau penyisipan papan ketik.",
            Language::German => "Verwenden Sie die Matcher-Vorschau unten im Fenster. Geben Sie den Auslöser genau ein; er sollte durch den erwarteten Text ersetzt werden. Das prüft Snippet und Matcher lokal, nicht Tastaturerfassung oder Einfügen.",
        }
    }

    pub fn onboarding_real_app_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Try it in a real app",
            Language::Indonesian => "Coba di aplikasi nyata",
            Language::German => "In einer echten App testen",
        }
    }

    pub fn onboarding_real_app_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Open a plain text field in an app you use (for example, a text editor). Type the trigger followed by a space. Confirm that only the trigger is replaced by the expected text. Never test in a password, terminal-credential, or other sensitive field.",
            Language::Indonesian => "Buka kolom teks biasa di aplikasi yang Anda gunakan (misalnya editor teks). Ketik pemicu lalu spasi. Pastikan hanya pemicu yang diganti dengan teks yang diharapkan. Jangan pernah menguji di kolom kata sandi, kredensial terminal, atau kolom sensitif lainnya.",
            Language::German => "Öffnen Sie ein normales Textfeld in einer App, etwa einem Texteditor. Geben Sie den Auslöser plus Leerzeichen ein. Prüfen Sie, ob nur der Auslöser durch den erwarteten Text ersetzt wird. Testen Sie nie in Passwort-, Terminal-Zugangsdaten- oder anderen sensiblen Feldern.",
        }
    }

    pub fn onboarding_expected_result(&self) -> &'static str {
        match self.lang {
            Language::English => "Expected result: WayExpand is working!",
            Language::Indonesian => "Hasil yang diharapkan: WayExpand is working!",
            Language::German => "Erwartetes Ergebnis: WayExpand is working!",
        }
    }

    pub fn onboarding_trigger(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Trigger: {trigger}"),
            Language::Indonesian => format!("Pemicu: {trigger}"),
            Language::German => format!("Auslöser: {trigger}"),
        }
    }

    pub fn onboarding_copy_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy trigger",
            Language::Indonesian => "Salin pemicu",
            Language::German => "Auslöser kopieren",
        }
    }

    pub fn onboarding_test_succeeded(&self) -> &'static str {
        match self.lang {
            Language::English => "It expanded correctly",
            Language::Indonesian => "Ekspansi berhasil",
            Language::German => "Ersetzung hat funktioniert",
        }
    }

    pub fn onboarding_backend_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Finish the session check before testing input in another app.",
            Language::Indonesian => "Selesaikan pemeriksaan sesi sebelum menguji masukan di aplikasi lain.",
            Language::German => "Schließen Sie die Sitzungsprüfung ab, bevor Sie die Eingabe in einer anderen App testen.",
        }
    }

    pub fn onboarding_test_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "It did not expand correctly",
            Language::Indonesian => "Ekspansi tidak berhasil",
            Language::German => "Ersetzung hat nicht funktioniert",
        }
    }

    pub fn onboarding_troubleshooting_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Let’s recover the test",
            Language::Indonesian => "Mari perbaiki pengujian",
            Language::German => "Test gemeinsam beheben",
        }
    }

    pub fn onboarding_troubleshooting_steps(&self) -> &'static str {
        match self.lang {
            Language::English => "1. Check that the daemon and route show Connected, then refresh session checks.\n2. If permission is required, approve the desktop input portal and refresh. Do not grant raw keyboard access just to make this test pass.\n3. Confirm the test snippet is enabled, then type :wayexpand-test followed by a space in an ordinary text field.\n4. If the matcher preview works but the real app does not, open diagnostics and check the active capture/injection route and focus support. Reload after changing the route.\n5. If text is partly deleted, duplicated, or inserted in the wrong place, pause WayExpand and do not repeat the test. Use a route with verified replacement before relying on it.",
            Language::Indonesian => "1. Pastikan daemon dan rute berstatus Terhubung, lalu segarkan pemeriksaan sesi.\n2. Jika izin diperlukan, setujui portal masukan desktop lalu segarkan. Jangan memberi akses papan ketik mentah hanya agar tes ini berhasil.\n3. Pastikan cuplikan uji aktif, lalu ketik :wayexpand-test diikuti spasi pada kolom teks biasa.\n4. Jika pratinjau pencocok berhasil tetapi aplikasi nyata tidak, buka diagnostik dan periksa rute tangkapan/penyisipan serta dukungan fokus. Muat ulang setelah mengubah rute.\n5. Jika teks terhapus sebagian, berulang, atau masuk ke tempat yang salah, jeda WayExpand dan jangan ulangi tes. Gunakan rute dengan penggantian terverifikasi sebelum mengandalkannya.",
            Language::German => "1. Prüfen Sie, ob Daemon und Route Verbunden anzeigen, und aktualisieren Sie die Sitzungsprüfung.\n2. Falls eine Freigabe nötig ist, bestätigen Sie das Desktop-Eingabeportal und aktualisieren Sie erneut. Gewähren Sie für diesen Test keinen Rohzugriff auf die Tastatur.\n3. Prüfen Sie, ob das Test-Snippet aktiv ist, und geben Sie :wayexpand-test plus Leerzeichen in ein normales Textfeld ein.\n4. Funktioniert die Matcher-Vorschau, aber nicht die App, öffnen Sie die Diagnose und prüfen Sie aktive Erfassungs-/Ausgaberoute sowie Fokusunterstützung. Laden Sie nach einem Routenwechsel neu.\n5. Werden Zeichen teilweise gelöscht, dupliziert oder falsch eingefügt, pausieren Sie WayExpand und wiederholen Sie den Test nicht. Verwenden Sie erst eine Route mit verifizierter Ersetzung.",
        }
    }

    pub fn onboarding_skip(&self) -> &'static str {
        match self.lang {
            Language::English => "Skip guide",
            Language::Indonesian => "Lewati panduan",
            Language::German => "Anleitung überspringen",
        }
    }

    pub fn getting_started(&self) -> &'static str {
        match self.lang {
            Language::English => "Getting started",
            Language::Indonesian => "Memulai",
            Language::German => "Erste Schritte",
        }
    }

    pub fn onboarding_complete_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Thanks for confirming. This records your manual test; WayExpand does not observe or certify insertion in another application.",
            Language::Indonesian => "Terima kasih atas konfirmasinya. Ini mencatat tes manual Anda; WayExpand tidak mengamati atau mensertifikasi penyisipan di aplikasi lain.",
            Language::German => "Danke für die Bestätigung. Damit wird Ihr manueller Test gespeichert; WayExpand beobachtet oder zertifiziert das Einfügen in anderen Apps nicht.",
        }
    }

    pub fn onboarding_skipped_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Setup guide skipped. Reopen it from More actions → Getting started.",
            Language::Indonesian => "Panduan penyiapan dilewati. Buka kembali dari Tindakan lainnya → Memulai.",
            Language::German => "Einrichtungsanleitung übersprungen. Über Weitere Aktionen → Erste Schritte erneut öffnen.",
        }
    }

    pub fn select_snippet_prompt(&self) -> &'static str {
        match self.lang {
            Language::English => "Select a snippet from your library to edit it.",
            Language::Indonesian => "Pilih cuplikan dari pustaka Anda untuk menyuntingnya.",
            Language::German => "Wählen Sie ein Snippet aus Ihrer Bibliothek zum Bearbeiten aus.",
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
