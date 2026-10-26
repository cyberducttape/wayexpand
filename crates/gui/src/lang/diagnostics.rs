use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn runtime_dashboard(&self) -> &'static str {
        match self.lang {
            Language::English => "Active session dashboard",
            Language::Indonesian => "Dasbor sesi aktif",
            Language::German => "Aktive Sitzungsübersicht",
        }
    }

    pub fn route_assessment_title(&self) -> &'static str {
        match self.lang {
            Language::English => "What this route can do",
            Language::Indonesian => "Kemampuan rute ini",
            Language::German => "Was diese Route kann",
        }
    }

    pub fn capture_availability_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Capture availability",
            Language::Indonesian => "Ketersediaan tangkapan",
            Language::German => "Verfügbarkeit der Erfassung",
        }
    }

    pub fn insertion_safety_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Insertion safety",
            Language::Indonesian => "Keamanan penyisipan",
            Language::German => "Sicherheit der Ausgabe",
        }
    }

    pub fn insertion_assurance_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "The daemon has not reported replacement guarantees.",
            Language::Indonesian => "Daemon belum melaporkan jaminan penggantian.",
            Language::German => "Der Daemon hat keine Ersetzungsgarantien gemeldet.",
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

    pub fn insertion_unicode_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "Full Unicode support has not been reported for this route.",
            Language::Indonesian => "Dukungan Unicode penuh belum dilaporkan untuk rute ini.",
            Language::German => "Volle Unicode-Unterstützung wurde für diese Route nicht gemeldet.",
        }
    }

    pub fn compositor_permissions_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Compositor permissions",
            Language::Indonesian => "Izin kompositor",
            Language::German => "Compositor-Berechtigungen",
        }
    }

    pub fn focus_visibility_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Focus visibility",
            Language::Indonesian => "Visibilitas fokus",
            Language::German => "Sichtbarkeit des Fokus",
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
    ) -> String {
        match (self.lang, state) {
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected)) => {
                format!("Active capture route {source} is connected.")
            }
            (Language::Indonesian, Some(wayexpand_core::DaemonRouteState::Connected)) => {
                format!("Rute tangkap aktif {source} terhubung.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected)) => {
                format!("Aktive Eingaberoute {source} ist verbunden.")
            }
            (_, Some(wayexpand_core::DaemonRouteState::PermissionRequired)) => {
                self.permission_required_route_detail(source)
            }
            (_, Some(wayexpand_core::DaemonRouteState::PortalRevoked)) => {
                self.portal_revoked_route_detail(source)
            }
            (
                Language::English,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
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
            ) => {
                format!("Eingaberoute {source} kann derzeit nicht ausgeführt werden.")
            }
            (Language::English, None) => {
                "The daemon has not reported an active capture route.".into()
            }
            (Language::Indonesian, None) => "Daemon belum melaporkan rute tangkap aktif.".into(),
            (Language::German, None) => "Der Daemon hat keine aktive Eingaberoute gemeldet.".into(),
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

    pub fn capture_route(&self) -> &'static str {
        match self.lang {
            Language::English => "Capture route",
            Language::Indonesian => "Rute tangkap",
            Language::German => "Eingaberoute",
        }
    }

    pub fn injection_backend(&self) -> &'static str {
        match self.lang {
            Language::English => "Injection backend",
            Language::Indonesian => "Backend penyisipan",
            Language::German => "Ausgabebackend",
        }
    }

    pub fn route_id(&self) -> &'static str {
        match self.lang {
            Language::English => "Route ID",
            Language::Indonesian => "ID rute",
            Language::German => "Routen-ID",
        }
    }

    pub fn compositor_version(&self) -> &'static str {
        match self.lang {
            Language::English => "Compositor",
            Language::Indonesian => "Kompositor",
            Language::German => "Compositor",
        }
    }

    pub fn permission_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Permission health",
            Language::Indonesian => "Status izin",
            Language::German => "Berechtigungsstatus",
        }
    }

    pub fn average_latency(&self) -> &'static str {
        match self.lang {
            Language::English => "Average injection latency",
            Language::Indonesian => "Latensi penyisipan rata-rata",
            Language::German => "Durchschnittliche Ausgabelatenz",
        }
    }

    pub fn sensitive_verification(&self) -> &'static str {
        match self.lang {
            Language::English => "Sensitive-field protection",
            Language::Indonesian => "Perlindungan kolom sensitif",
            Language::German => "Schutz sensibler Felder",
        }
    }

    pub fn recent_expansion_outcomes(&self) -> &'static str {
        match self.lang {
            Language::English => "Recent output outcomes · content-free trace",
            Language::Indonesian => "Hasil keluaran terbaru · jejak tanpa konten",
            Language::German => "Letzte Ausgabeergebnisse · inhaltsfreie Spur",
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
            (Language::English, "applied") => "Output transaction acknowledged".into(),
            (Language::Indonesian, "applied") => "Transaksi keluaran dikonfirmasi".into(),
            (Language::German, "applied") => "Ausgabetransaktion bestätigt".into(),
            (_, "inserted; cursor placement failed") => match self.lang {
                Language::English => "Text inserted; cursor placement failed".into(),
                Language::Indonesian => "Teks disisipkan; penempatan kursor gagal".into(),
                Language::German => "Text eingefügt; Cursorpositionierung fehlgeschlagen".into(),
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
            (_, "partial result unknown") => match self.lang {
                Language::English => "Partial output; final result is unknown".into(),
                Language::Indonesian => "Keluaran sebagian; hasil akhir tidak diketahui".into(),
                Language::German => "Teilausgabe; Endergebnis unbekannt".into(),
            },
            _ => completion.to_owned(),
        }
    }

    pub fn trace_trigger_matched(&self) -> &'static str {
        match self.lang {
            Language::English => "Trigger matched",
            Language::Indonesian => "Pemicu cocok",
            Language::German => "Trigger erkannt",
        }
    }

    pub fn trace_focus_admission(&self) -> &'static str {
        match self.lang {
            Language::English => "Focus admission",
            Language::Indonesian => "Pemeriksaan fokus",
            Language::German => "Fokusfreigabe",
        }
    }

    pub fn trace_backend_selected(&self) -> &'static str {
        match self.lang {
            Language::English => "Backend selected",
            Language::Indonesian => "Backend dipilih",
            Language::German => "Backend ausgewählt",
        }
    }

    pub fn trace_erase_acknowledgement(&self) -> &'static str {
        match self.lang {
            Language::English => "Erase/replacement acknowledgement",
            Language::Indonesian => "Konfirmasi penghapusan/penggantian",
            Language::German => "Bestätigung für Löschen/Ersetzen",
        }
    }

    pub fn trace_insertion_attempted(&self) -> &'static str {
        match self.lang {
            Language::English => "Insertion attempted",
            Language::Indonesian => "Penyisipan dicoba",
            Language::German => "Einfügen versucht",
        }
    }

    pub fn trace_privacy_and_limits(&self) -> &'static str {
        match self.lang {
            Language::English => "Only output transactions are recorded. Acknowledgement does not prove the target application displayed the text; unmatched typing is never recorded. Use Matcher preview to explain a test input.",
            Language::Indonesian => "Hanya transaksi keluaran yang dicatat. Konfirmasi tidak membuktikan aplikasi tujuan menampilkan teks; ketikan yang tidak cocok tidak pernah dicatat. Gunakan Pratinjau pencocok untuk menjelaskan teks uji.",
            Language::German => "Nur Ausgabetransaktionen werden erfasst. Eine Bestätigung beweist nicht, dass die Zielanwendung den Text angezeigt hat; nicht erkannte Eingaben werden nie protokolliert. Die Matcher-Vorschau erklärt Testeingaben.",
        }
    }

    pub fn trigger_suggestions(&self) -> &'static str {
        match self.lang {
            Language::English => "Available trigger suggestions",
            Language::Indonesian => "Saran pemicu yang tersedia",
            Language::German => "Verfügbare Trigger-Vorschläge",
        }
    }

    pub fn form_test_values(&self) -> &'static str {
        match self.lang {
            Language::English => "Test this form with sample values",
            Language::Indonesian => "Uji formulir dengan nilai contoh",
            Language::German => "Formular mit Beispielwerten testen",
        }
    }

    pub fn form_template_rendered(&self) -> &'static str {
        match self.lang {
            Language::English => "Rendered template",
            Language::Indonesian => "Templat hasil render",
            Language::German => "Gerenderte Vorlage",
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

    pub fn protection_uncertified(&self) -> &'static str {
        match self.lang {
            Language::English => "Not compositor-certified",
            Language::Indonesian => "Belum disertifikasi untuk kompositor",
            Language::German => "Für diesen Compositor nicht zertifiziert",
        }
    }

    pub fn fleet_layers(&self) -> &'static str {
        match self.lang {
            Language::English => "Fleet layers",
            Language::Indonesian => "Lapisan armada",
            Language::German => "Flotten-Ebenen",
        }
    }

    pub fn not_checked(&self) -> &'static str {
        match self.lang {
            Language::English => "Not checked",
            Language::Indonesian => "Belum diperiksa",
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
            (Language::Indonesian, BackendState::Available) => "Siap",
            (Language::English, BackendState::RequiresPermission) => "Needs permission",
            (Language::Indonesian, BackendState::RequiresPermission) => "Perlu izin",
            (Language::English, BackendState::Implemented) => "Not probed",
            (Language::Indonesian, BackendState::Implemented) => "Belum diperiksa",
            (Language::English, BackendState::Unavailable) => "Unavailable",
            (Language::Indonesian, BackendState::Unavailable) => "Tidak tersedia",
            (Language::English, BackendState::NotImplemented) => "Not implemented",
            (Language::Indonesian, BackendState::NotImplemented) => "Belum diimplementasikan",
            (Language::German, BackendState::Available) => "Bereit",
            (Language::German, BackendState::RequiresPermission) => "Berechtigung nötig",
            (Language::German, BackendState::Implemented) => "Nicht geprüft",
            (Language::German, BackendState::Unavailable) => "Nicht verfügbar",
            (Language::German, BackendState::NotImplemented) => "Nicht implementiert",
        }
    }
}
