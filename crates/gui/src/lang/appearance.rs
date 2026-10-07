use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn appearance(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance",
            Language::German => "Erscheinungsbild",
        }
    }

    pub fn engine(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing engine",
            Language::German => "Eingabe-Engine",
        }
    }

    pub fn appearance_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Appearance changes apply and are saved immediately.",
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
            Language::German => {
                "Diese Werte werden in der Konfigurationsdatei gespeichert. Zum Übernehmen auf Speichern klicken."
            }
        }
    }

    pub fn theme(&self) -> &'static str {
        match self.lang {
            Language::English => "Theme",
            Language::German => "Design",
        }
    }

    pub fn theme_dark(&self) -> &'static str {
        match self.lang {
            Language::English => "Dark",
            Language::German => "Dunkel",
        }
    }

    pub fn theme_light(&self) -> &'static str {
        match self.lang {
            Language::English => "Light",
            Language::German => "Hell",
        }
    }

    pub fn language(&self) -> &'static str {
        match self.lang {
            Language::English => "Language",
            Language::German => "Sprache",
        }
    }

    pub fn color_pack(&self) -> &'static str {
        match self.lang {
            Language::English => "Color pack",
            Language::German => "Farbschema",
        }
    }

    pub fn color_pack_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Pick a visual style for the editor.",
            Language::German => "Visuellen Stil für den Editor auswählen.",
        }
    }

    pub fn font_size(&self) -> &'static str {
        match self.lang {
            Language::English => "Font size",
            Language::German => "Schriftgröße",
        }
    }

    pub fn font_size_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Adjust text size for readability on your display.",
            Language::German => "Textgröße für die Lesbarkeit auf Ihrem Bildschirm anpassen.",
        }
    }

    pub fn font_scale_label(&self, scale: FontScale) -> &'static str {
        match (self.lang, scale) {
            (Language::English, FontScale::Small) => "Small (80%)",
            (Language::English, FontScale::Normal) => "Normal (100%)",
            (Language::English, FontScale::Large) => "Large (120%)",
            (Language::English, FontScale::ExtraLarge) => "Extra large (150%)",
            (Language::English, FontScale::Huge) => "Huge (200%)",
            (Language::German, FontScale::Small) => "Klein (80 %)",
            (Language::German, FontScale::Normal) => "Normal (100 %)",
            (Language::German, FontScale::Large) => "Groß (120 %)",
            (Language::German, FontScale::ExtraLarge) => "Sehr groß (150 %)",
            (Language::German, FontScale::Huge) => "Riesig (200 %)",
        }
    }
}
