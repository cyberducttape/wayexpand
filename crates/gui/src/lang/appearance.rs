use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn appearance_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance changes apply and are saved immediately.",
            Language::Indonesian => "Perubahan tampilan langsung diterapkan dan disimpan.",
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
            Language::Indonesian => {
                "Nilai-nilai ini disimpan di berkas konfigurasi Anda. Tekan Simpan untuk menerapkannya."
            }
            Language::German => {
                "Diese Werte werden in der Konfigurationsdatei gespeichert. Zum Übernehmen auf Speichern klicken."
            }
        }
    }

    pub fn font_scale_label(&self, scale: FontScale) -> &'static str {
        match (self.lang, scale) {
            (Language::English, FontScale::Small) => "Small (80%)",
            (Language::Indonesian, FontScale::Small) => "Kecil (80%)",
            (Language::English, FontScale::Normal) => "Normal (100%)",
            (Language::Indonesian, FontScale::Normal) => "Normal (100%)",
            (Language::English, FontScale::Large) => "Large (120%)",
            (Language::Indonesian, FontScale::Large) => "Besar (120%)",
            (Language::English, FontScale::ExtraLarge) => "Extra large (150%)",
            (Language::Indonesian, FontScale::ExtraLarge) => "Ekstra besar (150%)",
            (Language::English, FontScale::Huge) => "Huge (200%)",
            (Language::Indonesian, FontScale::Huge) => "Sangat besar (200%)",
            (Language::German, FontScale::Small) => "Klein (80 %)",
            (Language::German, FontScale::Normal) => "Normal (100 %)",
            (Language::German, FontScale::Large) => "Groß (120 %)",
            (Language::German, FontScale::ExtraLarge) => "Sehr groß (150 %)",
            (Language::German, FontScale::Huge) => "Riesig (200 %)",
        }
    }
}
