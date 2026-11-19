//! First-run guide and guided-setup text.

use super::{Language, Strings};

impl Strings {
    /// Describe the actual persisted consent mode. In particular, do not
    /// label `untracked-focus` as `tracked-focus`: users need to know which
    /// non-atomic replacement policy is already active before continuing.
    pub(crate) fn setup_consent_already(&self, mode: wayexpand_core::CompatibilityMode) -> String {
        let mode = self.compatibility_mode_option(mode);
        match self.lang {
            Language::English => {
                format!("Non-atomic text replacement is already enabled. Focus mode: {mode}.")
            }
            Language::German => format!(
                "Die nicht atomare Textersetzung ist bereits aktiviert. Fokusmodus: {mode}."
            ),
            Language::Indonesian => {
                format!("Penggantian teks non-atomik sudah aktif. Mode fokus: {mode}.")
            }
        }
    }
}

impl Strings {
    pub fn setup_unverified_state_unsaved(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("WayExpand is configured but unverified. The incomplete first-run state could not be saved, so the guide may not reopen automatically: {error}"),
            Language::German => format!("WayExpand ist eingerichtet, aber unverifiziert. Der offene Einrichtungszustand konnte nicht gespeichert werden, daher öffnet sich der Assistent eventuell nicht wieder von selbst: {error}"),
            Language::Indonesian => format!("WayExpand sudah dikonfigurasi tetapi belum terverifikasi. Status penggunaan pertama yang belum selesai tidak dapat disimpan, jadi panduan mungkin tidak terbuka lagi otomatis: {error}"),
        }
    }

    pub fn setup_unverified_fallback(&self, fallback: &str) -> String {
        match self.lang {
            Language::English => format!("{fallback} Configuration and service state were not enough to verify a real application insertion; check Diagnostics."),
            Language::German => format!("{fallback} Konfiguration und Dienststatus reichen nicht, um ein echtes Einfügen zu verifizieren; prüfen Sie die Diagnose."),
            Language::Indonesian => format!("{fallback} Konfigurasi dan status layanan tidak cukup untuk memverifikasi penyisipan di aplikasi nyata; periksa Diagnostik."),
        }
    }

    pub fn setup_could_not_check_stages(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Could not check the IBus setup stages: {detail}"),
            Language::German => {
                format!("Die IBus-Einrichtungsschritte konnten nicht geprüft werden: {detail}")
            }
            Language::Indonesian => format!("Tahap penyiapan IBus tidak dapat diperiksa: {detail}"),
        }
    }

    pub fn status_restarting_unit(&self, unit: &str) -> String {
        match self.lang {
            Language::English => format!("Restarting {unit}…"),
            Language::German => format!("{unit} wird neu gestartet…"),
            Language::Indonesian => format!("Memulai ulang {unit}…"),
        }
    }

    pub fn status_restart_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not start service restart: {error}"),
            Language::German => format!("Der Dienst konnte nicht neu gestartet werden: {error}"),
            Language::Indonesian => format!("Mulai ulang layanan tidak dapat dijalankan: {error}"),
        }
    }

    pub fn status_real_app_test_state_unsaved(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!(
                "A real-app test is still required, but the pending first-run state could not be saved: {error}"
            ),
            Language::German => format!(
                "Ein Test in einer echten Anwendung ist weiterhin nötig, aber der offene Einrichtungszustand konnte nicht gespeichert werden: {error}"
            ),
            Language::Indonesian => format!(
                "Uji di aplikasi nyata masih diperlukan, tetapi status penggunaan pertama yang tertunda tidak dapat disimpan: {error}"
            ),
        }
    }

    pub fn status_picker_executable_missing(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not locate the Quick Picker executable: {error}"),
            Language::German => {
                format!("Das Programm des Schnellwählers wurde nicht gefunden: {error}")
            }
            Language::Indonesian => format!("Program Quick Picker tidak ditemukan: {error}"),
        }
    }

    pub fn status_picker_open_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not open Quick Picker: {error}"),
            Language::German => format!("Der Schnellwähler konnte nicht geöffnet werden: {error}"),
            Language::Indonesian => format!("Quick Picker tidak dapat dibuka: {error}"),
        }
    }

    pub fn confirmed_in_application(&self, application: &str) -> String {
        match self.lang {
            Language::English => format!(
                "You confirmed the exact test replacement in {application}. This verifies that application and session only."
            ),
            Language::German => format!(
                "Sie haben die exakte Testersetzung in {application} bestätigt. Das verifiziert nur diese Anwendung und Sitzung."
            ),
            Language::Indonesian => format!(
                "Anda mengonfirmasi penggantian uji yang tepat di {application}. Ini hanya memverifikasi aplikasi dan sesi tersebut."
            ),
        }
    }

    pub fn machine_verified_in_application(&self, application: &str) -> String {
        match self.lang {
            Language::English => format!(
                "WayExpand observed the exact replacement in {application}. This verifies that controlled field and session only."
            ),
            Language::German => format!(
                "WayExpand hat die exakte Ersetzung in {application} beobachtet. Dies verifiziert nur dieses kontrollierte Feld und diese Sitzung."
            ),
            Language::Indonesian => format!(
                "WayExpand mengamati penggantian yang tepat di {application}. Ini hanya memverifikasi kolom terkontrol dan sesi tersebut."
            ),
        }
    }
}

impl Strings {
    pub(crate) fn form_field_error(&self, error: crate::app::FormFieldError) -> &'static str {
        use crate::app::FormFieldError as E;
        match (self.lang, error) {
            (Language::English, E::InvalidName) => "Use a field name beginning with a lowercase letter, followed by lowercase letters, digits, or underscores (up to 64 characters).",
            (Language::German, E::InvalidName) => "Verwenden Sie einen Feldnamen, der mit einem Kleinbuchstaben beginnt und danach nur Kleinbuchstaben, Ziffern oder Unterstriche enthält (bis zu 64 Zeichen).",
            (Language::Indonesian, E::InvalidName) => "Gunakan nama kolom yang diawali huruf kecil, diikuti huruf kecil, angka, atau garis bawah (hingga 64 karakter).",
            (Language::English, E::UnsafeDefault) => "The default contains characters that cannot be represented safely in a form marker.",
            (Language::German, E::UnsafeDefault) => "Der Standardwert enthält Zeichen, die sich in einer Formularmarkierung nicht sicher darstellen lassen.",
            (Language::Indonesian, E::UnsafeDefault) => "Nilai bawaan berisi karakter yang tidak dapat dinyatakan dengan aman dalam penanda formulir.",
            (Language::English, E::InvalidChoices) => "Enter 2–32 safe dropdown choices separated by commas.",
            (Language::German, E::InvalidChoices) => "Geben Sie 2–32 sichere Auswahlwerte ein, durch Kommas getrennt.",
            (Language::Indonesian, E::InvalidChoices) => "Masukkan 2–32 pilihan dropdown yang aman, dipisahkan koma.",
            (Language::English, E::DefaultNotAChoice) => "The dropdown default must exactly match one of its choices.",
            (Language::German, E::DefaultNotAChoice) => "Der Standardwert muss genau einem der Auswahlwerte entsprechen.",
            (Language::Indonesian, E::DefaultNotAChoice) => "Nilai bawaan dropdown harus sama persis dengan salah satu pilihannya.",
        }
    }
}

// The three-step first-run guide: connect, create, test.

// The optional Quick Picker shortcut step.
