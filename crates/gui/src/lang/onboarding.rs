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
