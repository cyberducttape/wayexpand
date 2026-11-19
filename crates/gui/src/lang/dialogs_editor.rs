use super::{Language, Strings};

impl Strings {
    pub fn profile_automatic_active(&self, name: &str) -> String {
        match self.lang {
            Language::English => format!("Application-selected profile: {name}"),
            Language::Indonesian => format!("Profil yang dipilih aplikasi: {name}"),
            Language::German => format!("Von der App ausgewähltes Profil: {name}"),
        }
    }

    pub fn save_before(&self, action: &str) -> String {
        match self.lang {
            Language::English => format!("Save changes before {}?", action),
            Language::Indonesian => format!("Simpan perubahan sebelum {}?", action),
            Language::German => format!("Änderungen vor {} speichern?", action),
        }
    }

    pub fn delete_confirm(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Delete this snippet? This cannot be recovered except through Undo."
            }
            Language::Indonesian => {
                "Hapus cuplikan ini? Tindakan ini hanya dapat dipulihkan melalui Urungkan."
            }
            Language::German => {
                "Dieses Snippet löschen? Dies kann nur über Rückgängig wiederhergestellt werden."
            }
        }
    }
}
