use super::{Language, Strings};

impl Strings {
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
