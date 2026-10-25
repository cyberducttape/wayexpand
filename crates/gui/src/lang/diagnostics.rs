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

    pub fn recent_expansion_failures(&self) -> &'static str {
        match self.lang {
            Language::English => "Recent expansion failures · content-free trace",
            Language::Indonesian => "Kegagalan ekspansi terbaru · jejak tanpa konten",
            Language::German => "Letzte Expansion-Fehler · inhaltsfreie Spur",
        }
    }

    pub fn no_recent_failures(&self) -> &'static str {
        match self.lang {
            Language::English => "No expansion failures recorded in this daemon session.",
            Language::Indonesian => "Tidak ada kegagalan ekspansi di sesi daemon ini.",
            Language::German => "In dieser Daemon-Sitzung wurden keine Expansion-Fehler erfasst.",
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
