use super::{Language, Strings};

impl Strings {
    pub fn unsaved_switching_profile(&self) -> &'static str {
        match self.lang {
            Language::English => "switching profiles",
            Language::Indonesian => "beralih profil",
            Language::German => "Profil wechseln",
        }
    }

    pub fn unsaved_creating_profile(&self) -> &'static str {
        match self.lang {
            Language::English => "creating a profile",
            Language::Indonesian => "membuat profil",
            Language::German => "Profil erstellen",
        }
    }

    pub fn profiles(&self) -> &'static str {
        match self.lang {
            Language::English => "Profiles",
            Language::Indonesian => "Profil",
            Language::German => "Profile",
        }
    }

    pub fn no_profiles(&self) -> &'static str {
        match self.lang {
            Language::English => "No profiles yet",
            Language::Indonesian => "Belum ada profil",
            Language::German => "Noch keine Profile",
        }
    }

    pub fn refresh_profiles(&self) -> &'static str {
        match self.lang {
            Language::English => "Refresh list",
            Language::Indonesian => "Muat ulang daftar",
            Language::German => "Liste aktualisieren",
        }
    }

    pub fn create_profile(&self) -> &'static str {
        match self.lang {
            Language::English => "Create and switch",
            Language::Indonesian => "Buat dan beralih",
            Language::German => "Erstellen und wechseln",
        }
    }

    pub fn profile_name_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "e.g. work",
            Language::Indonesian => "mis. kerja",
            Language::German => "z. B. arbeit",
        }
    }

    pub fn profile_loading(&self) -> &'static str {
        match self.lang {
            Language::English => "Loading profiles…",
            Language::Indonesian => "Memuat profil…",
            Language::German => "Profile werden geladen…",
        }
    }

    pub fn profile_invalid(&self) -> &'static str {
        match self.lang {
            Language::English => "invalid",
            Language::Indonesian => "tidak valid",
            Language::German => "ungültig",
        }
    }

    pub fn unsaved_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Unsaved changes",
            Language::Indonesian => "Perubahan belum disimpan",
            Language::German => "Ungespeicherte Änderungen",
        }
    }

    pub fn unsaved_switching(&self) -> &'static str {
        match self.lang {
            Language::English => "switching snippets",
            Language::Indonesian => "berpindah cuplikan",
            Language::German => "Snippets wechseln",
        }
    }

    pub fn unsaved_creating(&self) -> &'static str {
        match self.lang {
            Language::English => "creating a snippet",
            Language::Indonesian => "membuat cuplikan",
            Language::German => "ein Snippet erstellen",
        }
    }

    pub fn unsaved_duplicating(&self) -> &'static str {
        match self.lang {
            Language::English => "duplicating a snippet",
            Language::Indonesian => "menduplikasi cuplikan",
            Language::German => "ein Snippet duplizieren",
        }
    }

    pub fn unsaved_deleting(&self) -> &'static str {
        match self.lang {
            Language::English => "deleting a snippet",
            Language::Indonesian => "menghapus cuplikan",
            Language::German => "ein Snippet löschen",
        }
    }

    pub fn unsaved_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "reloading the configuration",
            Language::Indonesian => "memuat ulang konfigurasi",
            Language::German => "die Konfiguration neu laden",
        }
    }

    pub fn unsaved_undoing(&self) -> &'static str {
        match self.lang {
            Language::English => "undoing the last change",
            Language::Indonesian => "mengurungkan perubahan terakhir",
            Language::German => "die letzte Änderung rückgängig machen",
        }
    }

    pub fn unsaved_closing(&self) -> &'static str {
        match self.lang {
            Language::English => "closing WayExpand",
            Language::Indonesian => "menutup WayExpand",
            Language::German => "WayExpand schließen",
        }
    }

    pub fn save_before(&self, action: &str) -> String {
        match self.lang {
            Language::English => format!("Save changes before {}?", action),
            Language::Indonesian => format!("Simpan perubahan sebelum {}?", action),
            Language::German => format!("Änderungen vor {} speichern?", action),
        }
    }

    pub fn save_continue(&self) -> &'static str {
        match self.lang {
            Language::English => "Save and continue",
            Language::Indonesian => "Simpan dan lanjutkan",
            Language::German => "Speichern und fortfahren",
        }
    }

    pub fn discard(&self) -> &'static str {
        match self.lang {
            Language::English => "Discard",
            Language::Indonesian => "Buang",
            Language::German => "Verwerfen",
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

    pub fn delete_button(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete snippet",
            Language::Indonesian => "Hapus cuplikan",
            Language::German => "Snippet löschen",
        }
    }
}
