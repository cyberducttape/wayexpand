use super::{Language, Strings};
use crate::*;

impl Strings {
    /// One plain sentence at the top of diagnostics: whether expansion
    /// works right now and the one limitation that matters most.
    pub fn diagnostics_summary(
        &self,
        state: crate::lang::HeaderState,
        password_fields_detected: bool,
    ) -> &'static str {
        use crate::lang::HeaderState as S;
        match (self.lang, state, password_fields_detected) {
            (Language::English, S::NeedsVerification, _) => {
                "WayExpand is connected, but no expansion has been confirmed in a real application yet. Use Test in a real application."
            }
            (Language::English, S::Working | S::WorkingUnsaved, _) => {
                "Expansion is working. Password fields are detected where your desktop reports them."
            }
            (Language::English, S::Limited, false) => {
                "Expansion is working, but this setup cannot detect password fields. Pause WayExpand before typing secrets."
            }
            (Language::English, S::Limited, true) => {
                "Expansion is working with some safeguards unavailable on this setup. Technical details below."
            }
            (Language::English, S::Paused, _) => "Expansion is paused. Resume it from the menu.",
            (Language::English, S::Off | S::NotSetUp, _) => {
                "WayExpand is off. Turn it on from the header or the setup guide."
            }
            (Language::English, _, _) => {
                "Expansion is not working right now. The checks below show what needs attention."
            }
            (Language::German, S::NeedsVerification, _) => {
                "WayExpand ist verbunden, aber noch keine Ersetzung wurde in einer echten Anwendung bestätigt. Nutzen Sie „In einer echten Anwendung testen“."
            }
            (Language::German, S::Working | S::WorkingUnsaved, _) => {
                "Die Erweiterung funktioniert. Passwortfelder werden erkannt, wo der Desktop sie meldet."
            }
            (Language::German, S::Limited, false) => {
                "Die Erweiterung funktioniert, aber diese Einrichtung erkennt keine Passwortfelder. Pausieren Sie WayExpand, bevor Sie Geheimnisse eingeben."
            }
            (Language::German, S::Limited, true) => {
                "Die Erweiterung funktioniert, einige Schutzfunktionen fehlen aber in dieser Einrichtung. Technische Details unten."
            }
            (Language::German, S::Paused, _) => {
                "Die Erweiterung ist pausiert. Setzen Sie sie über das Menü fort."
            }
            (Language::German, S::Off | S::NotSetUp, _) => {
                "WayExpand ist aus. Schalten Sie es in der Kopfleiste oder im Einrichtungsassistenten ein."
            }
            (Language::German, _, _) => {
                "Die Erweiterung funktioniert gerade nicht. Die Prüfungen unten zeigen, was zu tun ist."
            }
            (Language::Indonesian, S::NeedsVerification, _) => {
                "WayExpand terhubung, tetapi belum ada ekspansi yang dipastikan di aplikasi nyata. Gunakan Uji di aplikasi nyata."
            }
            (Language::Indonesian, S::Working | S::WorkingUnsaved, _) => {
                "Ekspansi berfungsi. Kolom kata sandi dikenali bila desktop Anda melaporkannya."
            }
            (Language::Indonesian, S::Limited, false) => {
                "Ekspansi berfungsi, tetapi penyiapan ini tidak dapat mengenali kolom kata sandi. Jeda WayExpand sebelum mengetik rahasia."
            }
            (Language::Indonesian, S::Limited, true) => {
                "Ekspansi berfungsi dengan beberapa pengaman tidak tersedia di penyiapan ini. Detail teknis di bawah."
            }
            (Language::Indonesian, S::Paused, _) => {
                "Ekspansi dijeda. Lanjutkan dari menu."
            }
            (Language::Indonesian, S::Off | S::NotSetUp, _) => {
                "WayExpand mati. Nyalakan dari kepala jendela atau panduan penyiapan."
            }
            (Language::Indonesian, _, _) => {
                "Ekspansi sedang tidak berfungsi. Pemeriksaan di bawah menunjukkan apa yang perlu ditangani."
            }
        }
    }

    pub fn layout_assessment_detail(&self, layout_aware: Option<bool>) -> String {
        match (self.lang, layout_aware) {
            (_, None) => match self.lang {
                Language::English => "The active route has not reported keyboard-layout awareness.".into(),
                Language::Indonesian => "Rute aktif belum melaporkan kesadaran tata letak papan ketik.".into(),
                Language::German => "Die aktive Route meldet keine Informationen zur Tastaturlayout-Erkennung.".into(),
            },
            (_, Some(true)) => match self.lang {
                Language::English => "The route reports layout awareness; active layout changes still depend on the compositor and backend.".into(),
                Language::Indonesian => "Rute melaporkan dukungan tata letak; perubahan tata letak aktif tetap bergantung pada kompositor dan backend.".into(),
                Language::German => "Die Route meldet Layout-Unterstützung; Änderungen hängen weiterhin von Compositor und Backend ab.".into(),
            },
            (_, Some(false)) => match self.lang {
                Language::English => "This route cannot verify the active layout. Layout-dependent matching or synthetic output is refused instead of guessed; text-commit routes may not need keyboard-layout information.".into(),
                Language::Indonesian => "Rute ini tidak dapat memverifikasi tata letak aktif. Pencocokan atau keluaran sintetis yang bergantung pada tata letak ditolak; rute komit teks mungkin tidak memerlukan informasi tata letak.".into(),
                Language::German => "Diese Route kann das aktive Layout nicht verifizieren. Layoutabhängige Erkennung oder synthetische Ausgabe wird verweigert; Text-Commit-Routen benötigen möglicherweise keine Layoutdaten.".into(),
            },
        }
    }

    pub fn report_updated_minutes_ago(&self, minutes: u64) -> String {
        match self.lang {
            Language::English => format!("Session report updated {minutes} min ago · refresh before relying on it"),
            Language::Indonesian => format!("Laporan sesi diperbarui {minutes} mnt lalu · segarkan sebelum mengandalkannya"),
            Language::German => format!("Sitzungsbericht vor {minutes} Min. aktualisiert · vor der Verwendung aktualisieren"),
        }
    }

    pub fn report_not_run_yet(&self) -> &'static str {
        match self.lang {
            Language::English => "No session report yet · refresh to collect current evidence",
            Language::Indonesian => {
                "Belum ada laporan sesi · segarkan untuk mengumpulkan bukti saat ini"
            }
            Language::German => {
                "Noch kein Sitzungsbericht · aktualisieren, um aktuelle Daten zu erfassen"
            }
        }
    }

    pub fn route_certification_value(&self, certified: Option<bool>) -> String {
        match (self.lang, certified) {
            (_, None) => "Unknown".into(),
            (Language::English, Some(true)) => "Certified".into(),
            (Language::English, Some(false)) => "Not certified".into(),
            (Language::Indonesian, Some(true)) => "Tersertifikasi".into(),
            (Language::Indonesian, Some(false)) => "Belum tersertifikasi".into(),
            (Language::German, Some(true)) => "Zertifiziert".into(),
            (Language::German, Some(false)) => "Nicht zertifiziert".into(),
        }
    }

    pub fn next_step_daemon(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Start or restart the WayExpand user service, then refresh diagnostics."
            }
            Language::Indonesian => {
                "Mulai ulang layanan pengguna WayExpand, lalu segarkan diagnostik."
            }
            Language::German => {
                "Starten Sie den WayExpand-Benutzerdienst neu und aktualisieren Sie die Diagnose."
            }
        }
    }

    pub fn raw_capture_app_scope_notice(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!(
                "Raw-input matching is assigned to {count} configured applications. This limits where WayExpand matches, not where evdev observes keys; password fields cannot be detected, and browser-field insertion is not verified."
            ),
            Language::Indonesian => format!(
                "Pencocokan input mentah ditetapkan untuk {count} aplikasi. Ini membatasi lokasi WayExpand mencocokkan teks, bukan lokasi evdev mengamati tombol; kolom kata sandi tidak dapat dideteksi dan penyisipan di kolom browser belum diverifikasi."
            ),
            Language::German => format!(
                "Die Raw-Input-Suche ist {count} konfigurierten Anwendungen zugewiesen. Das begrenzt, wo WayExpand vergleicht, nicht wo evdev Tasten beobachtet; Passwortfelder werden nicht erkannt und das Einfügen in Browserfelder ist nicht verifiziert."
            ),
        }
    }

    pub fn capture_app_scope_state(&self, state: &str) -> &'static str {
        match (self.lang, state) {
            (_, "allowed") => match self.lang {
                Language::English => "Focused application matches the configured capture scope.",
                Language::Indonesian => "Aplikasi yang difokuskan cocok dengan cakupan tangkapan yang dikonfigurasi.",
                Language::German => "Die fokussierte Anwendung entspricht dem konfigurierten Erfassungsbereich.",
            },
            (_, "outside_allowlist") => match self.lang {
                Language::English => "Focused application is outside the capture allowlist; expansions are suppressed.",
                Language::Indonesian => "Aplikasi yang difokuskan tidak termasuk dalam daftar izin tangkapan; ekspansi ditekan.",
                Language::German => "Die fokussierte Anwendung steht nicht auf der Erfassungsliste; Expansionen werden unterdrückt.",
            },
            (_, "unrestricted") => match self.lang {
                Language::English => "This daemon route has no focused-application allowlist.",
                Language::Indonesian => "Rute daemon ini tidak memiliki daftar izin aplikasi yang difokuskan.",
                Language::German => "Diese Daemon-Route hat keine Allowlist für fokussierte Anwendungen.",
            },
            _ => match self.lang {
                Language::English => "Focused application is unknown; scoped expansion fails closed.",
                Language::Indonesian => "Aplikasi yang difokuskan tidak diketahui; ekspansi dengan cakupan diblokir secara aman.",
                Language::German => "Die fokussierte Anwendung ist unbekannt; begrenzte Expansionen werden sicher blockiert.",
            },
        }
    }

    pub fn evdev_quarantine_notice(&self, count: u64) -> String {
        match self.lang {
            Language::English => format!(
                "{count} captured input events were ignored after output to prevent stale input from triggering again. If a trigger did not expand, type it again after output settles."
            ),
            Language::Indonesian => format!(
                "{count} peristiwa input yang tertangkap diabaikan setelah output untuk mencegah input lama memicu ekspansi lagi. Jika pemicu tidak berekspansi, ketik ulang setelah output selesai."
            ),
            Language::German => format!(
                "{count} erfasste Eingabeereignisse wurden nach der Ausgabe verworfen, damit veraltete Eingaben keine erneute Expansion auslösen. Falls ein Kürzel nicht expandiert wurde, geben Sie es nach Abschluss der Ausgabe erneut ein."
            ),
        }
    }

    pub fn insertion_unicode_limited(&self) -> &'static str {
        match self.lang {
            Language::English => "Some Unicode text may not be supported by this route.",
            Language::Indonesian => "Beberapa teks Unicode mungkin tidak didukung rute ini.",
            Language::German => {
                "Einige Unicode-Zeichen werden von dieser Route möglicherweise nicht unterstützt."
            }
        }
    }

    pub fn assessment_state(&self, state: crate::diagnostics::AssessmentState) -> &'static str {
        use crate::diagnostics::AssessmentState::*;
        match (self.lang, state) {
            (Language::English, Ready) => "Ready",
            (Language::Indonesian, Ready) => "Siap",
            (Language::German, Ready) => "Bereit",
            (Language::English, Warning) => "Limited",
            (Language::Indonesian, Warning) => "Terbatas",
            (Language::German, Warning) => "Eingeschränkt",
            (Language::English, Blocked) => "Blocked",
            (Language::Indonesian, Blocked) => "Diblokir",
            (Language::German, Blocked) => "Blockiert",
            (Language::English, Unknown) => "Unknown",
            (Language::Indonesian, Unknown) => "Tidak diketahui",
            (Language::German, Unknown) => "Unbekannt",
        }
    }

    pub fn capture_assessment_detail(
        &self,
        state: Option<crate::runtime::RouteState>,
        source: &str,
        exclusive: Option<bool>,
    ) -> String {
        match (self.lang, state, exclusive) {
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), Some(true)) => {
                format!("Active capture route {source} is connected and captures keys exclusively.")
            }
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), Some(false)) => {
                format!("Active capture route {source} is connected but non-exclusive; keys reach the app before WayExpand can react.")
            }
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Active capture route {source} is connected; capture exclusivity is unreported.")
            }
            (
                Language::Indonesian,
                Some(wayexpand_core::DaemonRouteState::Connected),
                Some(true),
            ) => {
                format!(
                    "Rute tangkap aktif {source} terhubung dan menangkap tombol secara eksklusif."
                )
            }
            (
                Language::Indonesian,
                Some(wayexpand_core::DaemonRouteState::Connected),
                Some(false),
            ) => {
                format!("Rute tangkap aktif {source} terhubung tetapi tidak eksklusif; tombol mencapai aplikasi sebelum WayExpand dapat merespons.")
            }
            (Language::Indonesian, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Rute tangkap aktif {source} terhubung; status eksklusivitas tangkapan tidak dilaporkan.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), Some(true)) => {
                format!("Aktive Eingaberoute {source} ist verbunden und erfasst Tasten exklusiv.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), Some(false)) => {
                format!("Aktive Eingaberoute {source} ist verbunden, erfasst aber nicht exklusiv; Tasten erreichen die Anwendung, bevor WayExpand reagieren kann.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Aktive Eingaberoute {source} ist verbunden; die Exklusivität der Erfassung ist nicht bekannt.")
            }
            (_, Some(wayexpand_core::DaemonRouteState::PermissionRequired), _) => {
                self.permission_required_route_detail(source)
            }
            (_, Some(wayexpand_core::DaemonRouteState::PortalRevoked), _) => {
                self.portal_revoked_route_detail(source)
            }
            (
                Language::English,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Capture route {source} is not fully available yet.")
            }
            (
                Language::Indonesian,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Rute tangkap {source} belum sepenuhnya tersedia.")
            }
            (
                Language::German,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Eingaberoute {source} ist noch nicht vollständig verfügbar.")
            }
            (
                Language::English,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Capture route {source} cannot currently run.")
            }
            (
                Language::Indonesian,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Rute tangkap {source} tidak dapat berjalan saat ini.")
            }
            (
                Language::German,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Eingaberoute {source} kann derzeit nicht ausgeführt werden.")
            }
            (Language::English, None, _) => {
                "The daemon has not reported an active capture route.".into()
            }
            (Language::Indonesian, None, _) => "Daemon belum melaporkan rute tangkap aktif.".into(),
            (Language::German, None, _) => {
                "Der Daemon hat keine aktive Eingaberoute gemeldet.".into()
            }
        }
    }

    fn permission_required_route_detail(&self, source: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Capture route {source} is waiting for compositor or device permission.")
            }
            Language::Indonesian => {
                format!("Rute tangkap {source} menunggu izin kompositor atau perangkat.")
            }
            Language::German => {
                format!("Eingaberoute {source} wartet auf Compositor- oder Geräteberechtigung.")
            }
        }
    }

    fn portal_revoked_route_detail(&self, source: &str) -> String {
        match self.lang {
            Language::English => format!("Authorization for capture route {source} was revoked."),
            Language::Indonesian => format!("Otorisasi untuk rute tangkap {source} telah dicabut."),
            Language::German => {
                format!("Die Autorisierung für Eingaberoute {source} wurde widerrufen.")
            }
        }
    }

    pub fn focus_assessment_detail(
        &self,
        tracker: Option<bool>,
        exact_identity: Option<bool>,
    ) -> String {
        match (self.lang, tracker, exact_identity) {
            (Language::English, Some(true), Some(true)) => "Window tracker is connected and exposes exact toplevel identity.".into(),
            (Language::Indonesian, Some(true), Some(true)) => "Pelacak jendela terhubung dan memberikan identitas toplevel yang tepat.".into(),
            (Language::German, Some(true), Some(true)) => "Fenster-Tracker ist verbunden und liefert eine exakte Toplevel-Identität.".into(),
            (Language::English, Some(false), _) => "No window tracker is connected; application and focus changes are not visible.".into(),
            (Language::Indonesian, Some(false), _) => "Tidak ada pelacak jendela yang terhubung; perubahan aplikasi dan fokus tidak terlihat.".into(),
            (Language::German, Some(false), _) => "Kein Fenster-Tracker verbunden; Anwendungs- und Fokuswechsel sind nicht sichtbar.".into(),
            (Language::English, Some(true), Some(false)) => "Window changes are visible, but exact toplevel identity is unavailable.".into(),
            (Language::Indonesian, Some(true), Some(false)) => "Perubahan jendela terlihat, tetapi identitas toplevel yang tepat tidak tersedia.".into(),
            (Language::German, Some(true), Some(false)) => "Fensterwechsel sind sichtbar, aber die exakte Toplevel-Identität fehlt.".into(),
            (Language::English, _, _) => "Focus visibility has not been confirmed by the daemon.".into(),
            (Language::Indonesian, _, _) => "Visibilitas fokus belum dikonfirmasi oleh daemon.".into(),
            (Language::German, _, _) => "Die Fokus-Sichtbarkeit wurde vom Daemon nicht bestätigt.".into(),
        }
    }

    pub fn window_tracker_lifecycle_detail(
        &self,
        state: &str,
        reconnect_attempts: u64,
        disconnects: u64,
        last_failure_reason: &str,
        last_connected_age_seconds: Option<u64>,
        reconnect_backoff_ms: u64,
    ) -> String {
        let state = match (self.lang, state) {
            (Language::English, "starting") => "Starting",
            (Language::Indonesian, "starting") => "Memulai",
            (Language::German, "starting") => "Startet",
            (Language::English, "connected") => "Connected",
            (Language::Indonesian, "connected") => "Terhubung",
            (Language::German, "connected") => "Verbunden",
            (Language::English, "reconnecting") => "Reconnecting",
            (Language::Indonesian, "reconnecting") => "Menyambungkan ulang",
            (Language::German, "reconnecting") => "Verbindung wird erneut hergestellt",
            (Language::English, "stopped") => "Stopped",
            (Language::Indonesian, "stopped") => "Berhenti",
            (Language::German, "stopped") => "Angehalten",
            (Language::English, "unavailable") => "Unavailable",
            (Language::Indonesian, "unavailable") => "Tidak tersedia",
            (Language::German, "unavailable") => "Nicht verfügbar",
            (_, _) => "Unknown",
        };
        let last_failure_reason = match (self.lang, last_failure_reason) {
            (Language::English, "none") => "none",
            (Language::Indonesian, "none") => "tidak ada",
            (Language::German, "none") => "kein Fehler",
            (Language::English, "probe_unavailable") => "compositor service unavailable",
            (Language::Indonesian, "probe_unavailable") => "layanan kompositor tidak tersedia",
            (Language::German, "probe_unavailable") => "Compositor-Dienst nicht verfügbar",
            (Language::English, "connect_failed") => "connection failed",
            (Language::Indonesian, "connect_failed") => "koneksi gagal",
            (Language::German, "connect_failed") => "Verbindung fehlgeschlagen",
            (Language::English, "tracker_disconnected") => "connection lost",
            (Language::Indonesian, "tracker_disconnected") => "koneksi terputus",
            (Language::German, "tracker_disconnected") => "Verbindung verloren",
            (Language::English, "configuration_changed") => "restarting after a settings change",
            (Language::Indonesian, "configuration_changed") => {
                "dimulai ulang setelah perubahan pengaturan"
            }
            (Language::German, "configuration_changed") => {
                "Neustart nach einer Einstellungsänderung"
            }
            (Language::English, "compositor_unsupported") => "compositor is unsupported",
            (Language::Indonesian, "compositor_unsupported") => "kompositor tidak didukung",
            (Language::German, "compositor_unsupported") => "Compositor wird nicht unterstützt",
            (Language::English, "supervisor_start_failed") => "tracker worker could not start",
            (Language::Indonesian, "supervisor_start_failed") => "worker pelacak gagal dimulai",
            (Language::German, "supervisor_start_failed") => "Tracker-Worker konnte nicht starten",
            (Language::English, _) => "unknown",
            (Language::Indonesian, _) => "tidak diketahui",
            (Language::German, _) => "unbekannt",
        };
        let last_connected = match (self.lang, last_connected_age_seconds) {
            (_, None) => match self.lang {
                Language::English => "never connected".to_owned(),
                Language::Indonesian => "belum pernah terhubung".to_owned(),
                Language::German => "bisher nicht verbunden".to_owned(),
            },
            (Language::English, Some(0..=4)) => "just now".to_owned(),
            (Language::English, Some(seconds)) if seconds < 60 => {
                let unit = if seconds == 1 { "second" } else { "seconds" };
                format!("{seconds} {unit} ago")
            }
            (Language::English, Some(seconds)) if seconds < 3_600 => {
                let minutes = seconds / 60;
                let unit = if minutes == 1 { "minute" } else { "minutes" };
                format!("{minutes} {unit} ago")
            }
            (Language::English, Some(seconds)) => {
                let hours = seconds / 3_600;
                let unit = if hours == 1 { "hour" } else { "hours" };
                format!("{hours} {unit} ago")
            }
            (Language::Indonesian, Some(0..=4)) => "baru saja".to_owned(),
            (Language::Indonesian, Some(seconds)) if seconds < 60 => {
                format!("{} detik lalu", seconds.max(1))
            }
            (Language::Indonesian, Some(seconds)) if seconds < 3_600 => {
                format!("{} menit lalu", seconds / 60)
            }
            (Language::Indonesian, Some(seconds)) => {
                format!("{} jam lalu", seconds / 3_600)
            }
            (Language::German, Some(0..=4)) => "gerade eben".to_owned(),
            (Language::German, Some(seconds)) if seconds < 60 => {
                let unit = if seconds == 1 { "Sekunde" } else { "Sekunden" };
                format!("vor {seconds} {unit}")
            }
            (Language::German, Some(seconds)) if seconds < 3_600 => {
                let minutes = seconds / 60;
                let unit = if minutes == 1 { "Minute" } else { "Minuten" };
                format!("vor {minutes} {unit}")
            }
            (Language::German, Some(seconds)) => {
                let hours = seconds / 3_600;
                let unit = if hours == 1 { "Stunde" } else { "Stunden" };
                format!("vor {hours} {unit}")
            }
        };
        match self.lang {
            Language::English => format!(
                "Focus tracker: {state}; {reconnect_attempts} retries, {disconnects} disconnects; last issue: {last_failure_reason}; last connected {last_connected}; retry delay {reconnect_backoff_ms} ms."
            ),
            Language::Indonesian => format!(
                "Pelacak fokus: {state}; {reconnect_attempts} percobaan ulang, {disconnects} kali terputus; masalah terakhir: {last_failure_reason}; terakhir terhubung {last_connected}; jeda coba ulang {reconnect_backoff_ms} ms."
            ),
            Language::German => format!(
                "Fokus-Tracker: {state}; {reconnect_attempts} erneute Versuche, {disconnects} Verbindungsabbrüche; letzter Grund: {last_failure_reason}; zuletzt verbunden {last_connected}; Wiederholungswartezeit {reconnect_backoff_ms} ms."
            ),
        }
    }

    pub fn no_recent_outcomes(&self) -> &'static str {
        match self.lang {
            Language::English => "No output transactions recorded in this daemon session.",
            Language::Indonesian => {
                "Tidak ada transaksi keluaran yang tercatat di sesi daemon ini."
            }
            Language::German => {
                "In dieser Daemon-Sitzung wurden keine Ausgabetransaktionen erfasst."
            }
        }
    }

    pub fn trace_completion(&self, completion: &str) -> String {
        match (self.lang, completion) {
            (Language::English, "applied") => {
                "Output acknowledged; application text unverified".into()
            }
            (Language::Indonesian, "applied") => {
                "Keluaran dikonfirmasi; teks aplikasi belum diverifikasi".into()
            }
            (Language::German, "applied") => {
                "Ausgabe bestätigt; Anwendungstext nicht verifiziert".into()
            }
            (_, "inserted; cursor placement failed") => match self.lang {
                Language::English => {
                    "Output acknowledged; cursor placement failed; text unverified".into()
                }
                Language::Indonesian => {
                    "Keluaran dikonfirmasi; penempatan kursor gagal; teks belum diverifikasi".into()
                }
                Language::German => {
                    "Ausgabe bestätigt; Cursorpositionierung fehlgeschlagen; Text nicht verifiziert".into()
                }
            },
            (_, "not applied") => match self.lang {
                Language::English => "Output was not applied".into(),
                Language::Indonesian => "Keluaran tidak diterapkan".into(),
                Language::German => "Ausgabe wurde nicht angewendet".into(),
            },
            (_, "application outcome unknown") => match self.lang {
                Language::English => "Application outcome is unknown".into(),
                Language::Indonesian => "Hasil penerapan tidak diketahui".into(),
                Language::German => "Anwendungsergebnis ist unbekannt".into(),
            },
            (_, "partial_write" | "partial result unknown") => match self.lang {
                Language::English => "Partial write; check the target manually (not retried)".into(),
                Language::Indonesian => "Penulisan sebagian; periksa target secara manual (tidak diulang)".into(),
                Language::German => "Teilschreibvorgang; Ziel manuell prüfen (keine Wiederholung)".into(),
            },
            (_, "blocked by organization policy") => match self.lang {
                Language::English => "Blocked by organization policy before output".into(),
                Language::Indonesian => "Diblokir kebijakan organisasi sebelum keluaran".into(),
                Language::German => {
                    "Vor der Ausgabe durch Organisationsrichtlinie blockiert".into()
                }
            },
            (_, "blocked by command policy") => match self.lang {
                Language::English => "Blocked by command policy before output".into(),
                Language::Indonesian => "Diblokir kebijakan perintah sebelum keluaran".into(),
                Language::German => "Vor der Ausgabe durch Befehlsrichtlinie blockiert".into(),
            },
            (_, "key release was not confirmed before output") => match self.lang {
                Language::English => "Blocked because key release could not be confirmed".into(),
                Language::Indonesian => {
                    "Diblokir karena pelepasan tombol tidak dapat dikonfirmasi".into()
                }
                Language::German => {
                    "Blockiert, weil das Loslassen der Taste nicht bestätigt wurde".into()
                }
            },
            (_, "input quiet check failed") => match self.lang {
                Language::English => "Blocked because input safety could not be checked".into(),
                Language::Indonesian => {
                    "Diblokir karena keamanan input tidak dapat diperiksa".into()
                }
                Language::German => {
                    "Blockiert, weil die Eingabesicherheit nicht geprüft werden konnte".into()
                }
            },
            (_, "input arrived before output") => match self.lang {
                Language::English => "Blocked because more input arrived before replacement".into(),
                Language::Indonesian => "Diblokir karena ada input baru sebelum penggantian".into(),
                Language::German => {
                    "Blockiert, weil vor dem Ersetzen weitere Eingabe eintraf".into()
                }
            },
            (_, "multiple inputs arrived before output") => match self.lang {
                Language::English => {
                    "Blocked because multiple inputs arrived before replacement".into()
                }
                Language::Indonesian => {
                    "Diblokir karena beberapa input tiba sebelum penggantian".into()
                }
                Language::German => {
                    "Blockiert, weil vor dem Ersetzen mehrere Eingaben eintrafen".into()
                }
            },
            (_, "command timed out before output") => match self.lang {
                Language::English => "Command timed out before output".into(),
                Language::Indonesian => "Perintah kehabisan waktu sebelum keluaran".into(),
                Language::German => "Zeitüberschreitung des Befehls vor der Ausgabe".into(),
            },
            (_, "action broker unavailable") => match self.lang {
                Language::English => "Action Broker was unavailable".into(),
                Language::Indonesian => "Action Broker tidak tersedia".into(),
                Language::German => "Action Broker war nicht verfügbar".into(),
            },
            (_, "command output exceeded policy limit") => match self.lang {
                Language::English => "Command output exceeded the policy limit".into(),
                Language::Indonesian => "Keluaran perintah melebihi batas kebijakan".into(),
                Language::German => "Befehlsausgabe überschritt das Richtlinienlimit".into(),
            },
            (_, "command blocked by policy") => match self.lang {
                Language::English => "Command was blocked by policy".into(),
                Language::Indonesian => "Perintah diblokir oleh kebijakan".into(),
                Language::German => "Befehl wurde durch Richtlinie blockiert".into(),
            },
            (_, "command context was no longer safe") => match self.lang {
                Language::English => {
                    "Command result was discarded because the context became unsafe".into()
                }
                Language::Indonesian => {
                    "Hasil perintah dibuang karena konteks tidak lagi aman".into()
                }
                Language::German => {
                    "Befehlsergebnis verworfen, weil der Kontext nicht mehr sicher war".into()
                }
            },
            (_, "command result became stale") => match self.lang {
                Language::English => "Command result was discarded after input changed".into(),
                Language::Indonesian => "Hasil perintah dibuang setelah input berubah".into(),
                Language::German => "Befehlsergebnis nach Eingabeänderung verworfen".into(),
            },
            (_, "command focus could not be verified") => match self.lang {
                Language::English => {
                    "Command result was discarded because focus was uncertain".into()
                }
                Language::Indonesian => {
                    "Hasil perintah dibuang karena fokus tidak dapat dipastikan".into()
                }
                Language::German => "Befehlsergebnis wegen unklarem Fokus verworfen".into(),
            },
            (_, "command worker queue unavailable") => match self.lang {
                Language::English => "Command workers were busy or unavailable".into(),
                Language::Indonesian => "Worker perintah sibuk atau tidak tersedia".into(),
                Language::German => "Befehlsworker waren ausgelastet oder nicht verfügbar".into(),
            },
            (_, "command execution failed before output") => match self.lang {
                Language::English => "Command execution failed before output".into(),
                Language::Indonesian => "Eksekusi perintah gagal sebelum keluaran".into(),
                Language::German => "Befehlsausführung vor der Ausgabe fehlgeschlagen".into(),
            },
            (_, "expansion could not be prepared") => match self.lang {
                Language::English => "Expansion could not be prepared".into(),
                Language::Indonesian => "Ekspansi tidak dapat disiapkan".into(),
                Language::German => "Expansion konnte nicht vorbereitet werden".into(),
            },
            (_, "form requires verified replacement") => match self.lang {
                Language::English => "This snippet asks for form fields, but the active route cannot verify the trigger at the cursor. Use an input-method route with verified replacement, or remove the `{{field:...}}` marker.".into(),
                Language::Indonesian => "Cuplikan ini meminta isian formulir, tetapi rute aktif tidak dapat memverifikasi pemicu di kursor. Gunakan rute input-method dengan penggantian terverifikasi, atau hapus penanda `{{field:...}}`.".into(),
                Language::German => "Dieses Snippet fragt Formularfelder ab, aber die aktive Route kann den Trigger am Cursor nicht verifizieren. Verwenden Sie eine Input-Method-Route mit verifizierter Ersetzung oder entfernen Sie den Marker `{{field:...}}`.".into(),
            },
            (_, "form requires exact input-target identity") => match self.lang {
                Language::English => "This form needs an exact identity for the original text input, which the active route does not provide.".into(),
                Language::Indonesian => "Formulir ini memerlukan identitas tepat untuk input teks asal, tetapi rute aktif tidak menyediakannya.".into(),
                Language::German => "Dieses Formular benötigt eine genaue Identität für das ursprüngliche Texteingabefeld, die die aktive Route nicht bereitstellt.".into(),
            },
            (_, "form values could not render the template") => match self.lang {
                Language::English => "The submitted form values could not render this template. Check the snippet's variables and form fields.".into(),
                Language::Indonesian => "Nilai formulir yang dikirim tidak dapat merender templat ini. Periksa variabel dan isian formulir cuplikan.".into(),
                Language::German => "Die übermittelten Formularwerte konnten diese Vorlage nicht rendern. Prüfen Sie Variablen und Formularfelder des Snippets.".into(),
            },
            (_, "form submission values are invalid or incomplete") => match self.lang {
                Language::English => "The form submission was missing a value or contained a value that failed validation.".into(),
                Language::Indonesian => "Formulir tidak memiliki nilai wajib atau berisi nilai yang gagal divalidasi.".into(),
                Language::German => "Bei der Formularübermittlung fehlte ein Wert oder ein Wert hat die Prüfung nicht bestanden.".into(),
            },
            (_, "form helper output stream did not close") => match self.lang {
                Language::English => "The form helper did not finish its response. Check the GUI helper process and try again.".into(),
                Language::Indonesian => "Proses bantuan formulir tidak menyelesaikan responsnya. Periksa proses pembantu GUI lalu coba lagi.".into(),
                Language::German => "Der Formularhelfer hat seine Antwort nicht abgeschlossen. Prüfen Sie den GUI-Hilfsprozess und versuchen Sie es erneut.".into(),
            },
            _ => completion.to_owned(),
        }
    }

    pub fn trace_application_result(&self, result: &str) -> String {
        match (self.lang, result) {
            (Language::English, "verified") => "Target text verified".into(),
            (Language::Indonesian, "verified") => "Teks target terverifikasi".into(),
            (Language::German, "verified") => "Zieltext verifiziert".into(),
            (Language::English, "unverified") => "Target text not verified".into(),
            (Language::Indonesian, "unverified") => "Teks target belum diverifikasi".into(),
            (Language::German, "unverified") => "Zieltext nicht verifiziert".into(),
            (Language::English, "not_applied") => {
                "Output was rejected; target text was not changed".into()
            }
            (Language::Indonesian, "not_applied") => {
                "Keluaran ditolak; teks target tidak berubah".into()
            }
            (Language::German, "not_applied") => {
                "Ausgabe abgelehnt; Zieltext wurde nicht geändert".into()
            }
            (Language::English, "unknown") => "Target text may be partial or uncertain".into(),
            (Language::Indonesian, "unknown") => {
                "Teks target mungkin sebagian atau tidak pasti".into()
            }
            (Language::German, "unknown") => {
                "Zieltext möglicherweise teilweise oder ungewiss".into()
            }
            (Language::English, "not_attempted") => "No output was attempted".into(),
            (Language::Indonesian, "not_attempted") => "Tidak ada keluaran yang dicoba".into(),
            (Language::German, "not_attempted") => "Keine Ausgabe versucht".into(),
            (Language::English, _) => "Target text not verified".into(),
            (Language::Indonesian, _) => "Teks target belum diverifikasi".into(),
            (Language::German, _) => "Zieltext nicht verifiziert".into(),
        }
    }

    pub fn trace_timing(&self, duration_us: Option<u64>, expected_units: Option<usize>) -> String {
        let duration = duration_us.map_or_else(
            || match self.lang {
                Language::English => "timing unavailable".to_owned(),
                Language::Indonesian => "durasi tidak tersedia".to_owned(),
                Language::German => "Dauer nicht verfügbar".to_owned(),
            },
            |value| match self.lang {
                Language::English => format!("backend acknowledgement in {value} µs"),
                Language::Indonesian => format!("pengakuan backend dalam {value} µs"),
                Language::German => format!("Backend-Bestätigung in {value} µs"),
            },
        );
        let units = expected_units.map_or_else(
            || match self.lang {
                Language::English => "expected output size unavailable".to_owned(),
                Language::Indonesian => "ukuran keluaran yang diharapkan tidak tersedia".to_owned(),
                Language::German => "erwartete Ausgabegröße nicht verfügbar".to_owned(),
            },
            |value| match self.lang {
                Language::English => format!("{value} expected output units"),
                Language::Indonesian => format!("{value} unit keluaran yang diharapkan"),
                Language::German => format!("{value} erwartete Ausgabeeinheiten"),
            },
        );
        match self.lang {
            Language::English => format!("{units} · {duration}; not visible-text latency"),
            Language::Indonesian => format!("{units} · {duration}; bukan latensi teks terlihat"),
            Language::German => format!("{units} · {duration}; keine sichtbare Textlatenz"),
        }
    }

    pub fn trace_interrupt_timing(&self, duration_us: Option<u64>) -> Option<String> {
        duration_us.map(|value| match self.lang {
            Language::English => {
                format!("overlap observed → backend returned in {value} µs; client result unverified")
            }
            Language::Indonesian => format!(
                "tumpang tindih terdeteksi → backend kembali dalam {value} µs; hasil klien belum diverifikasi"
            ),
            Language::German => format!(
                "Überlappung erkannt → Backend-Rückkehr nach {value} µs; Client-Ergebnis nicht verifiziert"
            ),
        })
    }

    pub fn trace_progress(&self, erased: usize, inserted: usize) -> String {
        match self.lang {
            Language::English => format!("Confirmed output progress: {erased} erased, {inserted} inserted units"),
            Language::Indonesian => format!("Progres keluaran terkonfirmasi: {erased} unit dihapus, {inserted} unit disisipkan"),
            Language::German => format!("Bestätigter Ausgabefortschritt: {erased} gelöscht, {inserted} eingefügte Einheiten"),
        }
    }

    pub fn form_preview_privacy(&self) -> &'static str {
        match self.lang {
            Language::English => "Values and rendered text stay in this editor session.",
            Language::Indonesian => {
                "Nilai dan teks hasil render hanya tersimpan di sesi editor ini."
            }
            Language::German => "Werte und gerenderter Text bleiben in dieser Editorsitzung.",
        }
    }

    pub fn cursor_placement(&self, position: usize) -> String {
        match self.lang {
            Language::English => format!("Cursor placement: character {position}"),
            Language::Indonesian => format!("Posisi kursor: karakter {position}"),
            Language::German => format!("Cursorposition: Zeichen {position}"),
        }
    }

    pub fn template_error(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Template error: {error}"),
            Language::Indonesian => format!("Kesalahan templat: {error}"),
            Language::German => format!("Vorlagenfehler: {error}"),
        }
    }

    pub fn match_explanation(&self, would_expand: bool) -> &'static str {
        match (self.lang, would_expand) {
            (Language::English, true) => "Why this input would expand",
            (Language::English, false) => "Why this input would not expand",
            (Language::Indonesian, true) => "Mengapa teks ini akan diperluas",
            (Language::Indonesian, false) => "Mengapa teks ini tidak akan diperluas",
            (Language::German, true) => "Warum diese Eingabe expandiert würde",
            (Language::German, false) => "Warum diese Eingabe nicht expandiert würde",
        }
    }

    /// A readable label for a backend's state. The raw
    /// `implementation/availability/permission` triple is still shown
    /// verbatim next to it so the GUI stays a faithful mirror of
    /// `wayexpand doctor` rather than paraphrasing it away.
    pub fn backend_state(&self, state: BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English, BackendState::Available) => "Ready",
            (Language::Indonesian, BackendState::Available) => "Siap",
            (Language::English, BackendState::RequiresPermission) => "Needs permission",
            (Language::Indonesian, BackendState::RequiresPermission) => "Perlu izin",
            (Language::English, BackendState::ActionRequired) => "Action needed",
            (Language::Indonesian, BackendState::ActionRequired) => "Perlu tindakan",
            (Language::English, BackendState::Implemented) => "Not probed",
            (Language::Indonesian, BackendState::Implemented) => "Belum diperiksa",
            (Language::English, BackendState::Unavailable) => "Unavailable",
            (Language::Indonesian, BackendState::Unavailable) => "Tidak tersedia",
            (Language::English, BackendState::NotImplemented) => "Not implemented",
            (Language::Indonesian, BackendState::NotImplemented) => "Belum diimplementasikan",
            (Language::German, BackendState::Available) => "Bereit",
            (Language::German, BackendState::RequiresPermission) => "Berechtigung nötig",
            (Language::German, BackendState::ActionRequired) => "Aktion erforderlich",
            (Language::German, BackendState::Implemented) => "Nicht geprüft",
            (Language::German, BackendState::Unavailable) => "Nicht verfügbar",
            (Language::German, BackendState::NotImplemented) => "Nicht implementiert",
        }
    }

    pub fn ibus_route_ready(&self) -> &'static str {
        match self.lang {
            Language::English => "Ready: IBus can expand in apps that report their text.",
            Language::Indonesian => {
                "Siap: IBus dapat mengekspansi di aplikasi yang melaporkan teksnya."
            }
            Language::German => {
                "Bereit: IBus kann in Anwendungen erweitern, die ihren Text melden."
            }
        }
    }

    pub fn ibus_route_not_ready(&self, failed_check: &str) -> String {
        match self.lang {
            Language::English => format!("Not ready — fix this first: {failed_check}"),
            Language::Indonesian => format!("Belum siap — perbaiki ini dulu: {failed_check}"),
            Language::German => format!("Nicht bereit — zuerst beheben: {failed_check}"),
        }
    }

    pub fn ibus_field_counts(&self, keys: u64, output_emissions: u64) -> String {
        match self.lang {
            Language::English => {
                format!("{keys} keys · {output_emissions} output emission(s), unverified")
            }
            Language::Indonesian => {
                format!("{keys} tombol · {output_emissions} keluaran dikirim, belum terverifikasi")
            }
            Language::German => {
                format!("{keys} Tasten · {output_emissions} Ausgabe(n) gesendet, nicht verifiziert")
            }
        }
    }
    pub fn ibus_readiness_stage(&self, stage: &str) -> &'static str {
        match (stage, self.lang) {
            ("component_installed", Language::English) => "component installed",
            ("component_installed", Language::Indonesian) => "komponen terpasang",
            ("component_installed", Language::German) => "Komponente installiert",
            ("registered", Language::English) => "registered with IBus",
            ("registered", Language::Indonesian) => "terdaftar di IBus",
            ("registered", Language::German) => "bei IBus registriert",
            ("ibus_running", Language::English) => "IBus running",
            ("ibus_running", Language::Indonesian) => "IBus berjalan",
            ("ibus_running", Language::German) => "IBus läuft",
            ("selected", Language::English) => "selected",
            ("selected", Language::Indonesian) => "dipilih",
            ("selected", Language::German) => "ausgewählt",
            ("engine_running", Language::English) => "engine running",
            ("engine_running", Language::Indonesian) => "mesin berjalan",
            ("engine_running", Language::German) => "Engine läuft",
            ("text_received", Language::English) => "text received",
            ("text_received", Language::Indonesian) => "teks diterima",
            ("text_received", Language::German) => "Text empfangen",
            ("replacement_context", Language::English) => "replacement context observed",
            ("replacement_context", Language::Indonesian) => "konteks penggantian teramati",
            ("replacement_context", Language::German) => "Ersetzungskontext erkannt",
            ("replacement-context-observed", Language::English) => "replacement context observed",
            ("replacement-context-observed", Language::Indonesian) => {
                "konteks penggantian teramati"
            }
            ("replacement-context-observed", Language::German) => "Ersetzungskontext erkannt",
            ("output-emitted-unverified", Language::English) => "output emitted (unverified)",
            ("output-emitted-unverified", Language::Indonesian) => {
                "keluaran dikirim (belum terverifikasi)"
            }
            ("output-emitted-unverified", Language::German) => {
                "Ausgabe gesendet (nicht verifiziert)"
            }
            ("output_emitted_unverified", Language::English) => "output emitted (unverified)",
            ("output_emitted_unverified", Language::Indonesian) => {
                "keluaran dikirim (belum terverifikasi)"
            }
            ("output_emitted_unverified", Language::German) => {
                "Ausgabe gesendet (nicht verifiziert)"
            }
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_capture_scope_notice_explains_app_matching_and_global_observation() {
        let english = Strings::new(Language::English).raw_capture_app_scope_notice(14);
        assert!(english.contains("14 configured applications"));
        assert!(english.contains("not where evdev observes keys"));
        assert!(english.contains("password fields cannot be detected"));
        assert!(english.contains("browser-field insertion is not verified"));
        assert!(!english.contains("firefox"));

        assert!(Strings::new(Language::Indonesian)
            .raw_capture_app_scope_notice(2)
            .contains("2 aplikasi"));
        assert!(Strings::new(Language::German)
            .raw_capture_app_scope_notice(3)
            .contains("3 konfigurierten Anwendungen"));

        assert!(Strings::new(Language::English)
            .capture_app_scope_state("unknown")
            .contains("fails closed"));
        assert!(Strings::new(Language::German)
            .capture_app_scope_state("outside_allowlist")
            .contains("nicht auf der Erfassungsliste"));

        for language in [Language::English, Language::Indonesian, Language::German] {
            let message = Strings::new(language).compositor_surface_focus_diagnostic();
            assert!(message.contains("WX-EVDEV-FOCUS-006"));
            assert!(message.contains("kwin_wayland"));
            assert!(!message.contains("2720"));
        }
    }

    #[test]
    fn focus_tracker_lifecycle_detail_humanizes_stable_codes() {
        let english = Strings::new(Language::English).window_tracker_lifecycle_detail(
            "reconnecting",
            3,
            2,
            "tracker_disconnected",
            Some(120),
            1_000,
        );
        assert!(english.contains("Reconnecting"));
        assert!(english.contains("connection lost"));
        assert!(english.contains("2 minutes ago"));
        assert!(!english.contains("tracker_disconnected"));

        let indonesian = Strings::new(Language::Indonesian).window_tracker_lifecycle_detail(
            "connected",
            0,
            0,
            "none",
            None,
            0,
        );
        assert!(indonesian.contains("Terhubung"));
        assert!(indonesian.contains("belum pernah terhubung"));
        assert_ne!(indonesian, english);

        let german = Strings::new(Language::German).window_tracker_lifecycle_detail(
            "unavailable",
            1,
            0,
            "compositor_unsupported",
            Some(0),
            250,
        );
        assert!(german.contains("Nicht verfügbar"));
        assert!(german.contains("Compositor wird nicht unterstützt"));
        assert!(german.contains("gerade eben"));
    }

    #[test]
    fn input_method_progress_label_is_desktop_neutral_and_localized() {
        let english = Strings::new(Language::English).input_method_progress_title();
        let indonesian = Strings::new(Language::Indonesian).input_method_progress_title();
        let german = Strings::new(Language::German).input_method_progress_title();

        assert!(english.starts_with("Input method:"));
        assert!(!english.contains("GNOME"));
        assert!(indonesian.starts_with("Metode input:"));
        assert!(german.starts_with("Eingabemethode:"));
    }

    #[test]
    fn expansion_failure_reasons_are_translated() {
        let labels = [
            (
                "command timed out before output",
                "Perintah kehabisan waktu sebelum keluaran",
                "Zeitüberschreitung des Befehls vor der Ausgabe",
            ),
            (
                "command output exceeded policy limit",
                "Keluaran perintah melebihi batas kebijakan",
                "Befehlsausgabe überschritt das Richtlinienlimit",
            ),
            (
                "command result became stale",
                "Hasil perintah dibuang setelah input berubah",
                "Befehlsergebnis nach Eingabeänderung verworfen",
            ),
            (
                "command context was no longer safe",
                "Hasil perintah dibuang karena konteks tidak lagi aman",
                "Befehlsergebnis verworfen, weil der Kontext nicht mehr sicher war",
            ),
            (
                "form requires verified replacement",
                "Cuplikan ini meminta isian formulir, tetapi rute aktif tidak dapat memverifikasi pemicu di kursor. Gunakan rute input-method dengan penggantian terverifikasi, atau hapus penanda `{{field:...}}`.",
                "Dieses Snippet fragt Formularfelder ab, aber die aktive Route kann den Trigger am Cursor nicht verifizieren. Verwenden Sie eine Input-Method-Route mit verifizierter Ersetzung oder entfernen Sie den Marker `{{field:...}}`.",
            ),
            (
                "form values could not render the template",
                "Nilai formulir yang dikirim tidak dapat merender templat ini. Periksa variabel dan isian formulir cuplikan.",
                "Die übermittelten Formularwerte konnten diese Vorlage nicht rendern. Prüfen Sie Variablen und Formularfelder des Snippets.",
            ),
            (
                "form submission values are invalid or incomplete",
                "Formulir tidak memiliki nilai wajib atau berisi nilai yang gagal divalidasi.",
                "Bei der Formularübermittlung fehlte ein Wert oder ein Wert hat die Prüfung nicht bestanden.",
            ),
            (
                "form helper output stream did not close",
                "Proses bantuan formulir tidak menyelesaikan responsnya. Periksa proses pembantu GUI lalu coba lagi.",
                "Der Formularhelfer hat seine Antwort nicht abgeschlossen. Prüfen Sie den GUI-Hilfsprozess und versuchen Sie es erneut.",
            ),
            (
                "input arrived before output",
                "Diblokir karena ada input baru sebelum penggantian",
                "Blockiert, weil vor dem Ersetzen weitere Eingabe eintraf",
            ),
        ];

        for (key, indonesian, german) in labels {
            assert_eq!(
                Strings::new(Language::Indonesian).trace_completion(key),
                indonesian
            );
            assert_eq!(Strings::new(Language::German).trace_completion(key), german);
            assert_ne!(Strings::new(Language::English).trace_completion(key), key);
        }
    }

    #[test]
    fn trace_application_result_never_implies_unverified_output_was_applied() {
        assert_eq!(
            Strings::new(Language::English).trace_application_result("unverified"),
            "Target text not verified"
        );
        assert_eq!(
            Strings::new(Language::English).trace_application_result("unknown"),
            "Target text may be partial or uncertain"
        );
        assert_eq!(
            Strings::new(Language::English).trace_application_result("not_attempted"),
            "No output was attempted"
        );
        assert_eq!(
            Strings::new(Language::English).trace_application_result("not_applied"),
            "Output was rejected; target text was not changed"
        );
        for language in [Language::Indonesian, Language::German] {
            for state in [
                "verified",
                "unverified",
                "unknown",
                "not_attempted",
                "not_applied",
            ] {
                let label = Strings::new(language).trace_application_result(state);
                assert!(!label.is_empty());
                assert_ne!(label, state);
            }
        }
    }

    #[test]
    fn trace_timing_labels_backend_acknowledgement_as_not_visible_text_latency() {
        let english = Strings::new(Language::English).trace_timing(Some(42), Some(5));
        assert!(english.contains("42 µs"));
        assert!(english.contains("5 expected output units"));
        assert!(english.contains("not visible-text latency"));
        assert!(Strings::new(Language::Indonesian)
            .trace_timing(None, None)
            .contains("bukan latensi teks terlihat"));
        assert!(Strings::new(Language::German)
            .trace_timing(None, None)
            .contains("keine sichtbare Textlatenz"));
    }
}
