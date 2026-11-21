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

    pub(crate) fn desktop_compatibility_heading(&self) -> &'static str {
        match self.lang {
            Language::English => "Desktop compatibility before setup",
            Language::German => "Desktop-Kompatibilität vor der Einrichtung",
            Language::Indonesian => "Kompatibilitas desktop sebelum penyiapan",
        }
    }

    /// Current likely routes and evidence, shown before setup can request
    /// desktop integration or permissions. Headless coverage is never labeled
    /// as real-session certification.
    pub(crate) fn desktop_compatibility_rows(&self) -> [(&'static str, &'static str); 5] {
        match self.lang {
            Language::English => [
                (
                    "GNOME 46, 48, 50, 51",
                    "IBus + WayExpand extension; headless-tested, not certified on a real desktop.",
                ),
                (
                    "GNOME 49",
                    "Extension support is undeclared; German/French layout switching is unverified in a real session. Quick Picker can copy for manual paste.",
                ),
                (
                    "Other GNOME Shell majors",
                    "Extension support is undeclared and compatibility is unknown. Check the setup guide before enabling an integration; Quick Picker can copy for manual paste.",
                ),
                (
                    "KDE Plasma / Wayland",
                    "IBus setup is available; verify in each target app. No route is production-certified.",
                ),
                (
                    "Sway, Hyprland, other desktops",
                    "Experimental or session-dependent. Setup checks what is available; Quick Picker can copy for manual paste.",
                ),
            ],
            Language::German => [
                (
                    "GNOME 46, 48, 50, 51",
                    "IBus + WayExpand-Erweiterung; headless getestet, nicht auf einem echten Desktop zertifiziert.",
                ),
                (
                    "GNOME 49",
                    "Erweiterungsunterstützung ist nicht deklariert; deutsche/französische Layoutwechsel sind in einer realen Sitzung ungeprüft. Der Schnellwähler kann zum manuellen Einfügen kopieren.",
                ),
                (
                    "Andere GNOME-Shell-Hauptversionen",
                    "Erweiterungsunterstützung ist nicht deklariert und die Kompatibilität unbekannt. Prüfen Sie die Einrichtungsanleitung, bevor Sie eine Integration aktivieren; der Schnellwähler kann zum manuellen Einfügen kopieren.",
                ),
                (
                    "KDE Plasma / Wayland",
                    "IBus-Einrichtung ist verfügbar; testen Sie jede Zielanwendung. Kein Eingabeweg ist produktionszertifiziert.",
                ),
                (
                    "Sway, Hyprland, andere Desktops",
                    "Experimentell oder sitzungsabhängig. Die Einrichtung prüft verfügbare Wege; der Schnellwähler kann zum manuellen Einfügen kopieren.",
                ),
            ],
            Language::Indonesian => [
                (
                    "GNOME 46, 48, 50, 51",
                    "IBus + ekstensi WayExpand; diuji headless, belum tersertifikasi pada desktop nyata.",
                ),
                (
                    "GNOME 49",
                    "Dukungan ekstensi belum dinyatakan; pergantian tata letak Jerman/Prancis belum terverifikasi pada sesi nyata. Quick Picker dapat menyalin untuk ditempel manual.",
                ),
                (
                    "Versi utama GNOME Shell lainnya",
                    "Dukungan ekstensi belum dinyatakan dan kompatibilitas belum diketahui. Periksa panduan penyiapan sebelum mengaktifkan integrasi; Quick Picker dapat menyalin untuk ditempel manual.",
                ),
                (
                    "KDE Plasma / Wayland",
                    "Penyiapan IBus tersedia; uji di setiap aplikasi tujuan. Belum ada rute yang tersertifikasi untuk produksi.",
                ),
                (
                    "Sway, Hyprland, desktop lain",
                    "Eksperimental atau bergantung pada sesi. Penyiapan memeriksa rute yang tersedia; Quick Picker dapat menyalin untuk ditempel manual.",
                ),
            ],
        }
    }

    pub(crate) fn desktop_compatibility_version_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "To check your GNOME Shell version, run `gnome-shell --version`.",
            Language::German => "Ihre GNOME-Shell-Version prüfen Sie mit `gnome-shell --version`.",
            Language::Indonesian => {
                "Periksa versi GNOME Shell dengan menjalankan `gnome-shell --version`."
            }
        }
    }

    pub(crate) fn desktop_compatibility_detected_version(&self, version: &str) -> String {
        match self.lang {
            Language::English => format!("Detected GNOME Shell version: {version}."),
            Language::German => format!("Erkannte GNOME-Shell-Version: {version}."),
            Language::Indonesian => format!("Versi GNOME Shell terdeteksi: {version}."),
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
