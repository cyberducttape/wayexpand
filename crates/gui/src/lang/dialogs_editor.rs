use super::{Language, Strings};

impl Strings {
    pub fn unsaved_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Unsaved changes",
            Language::German => "Ungespeicherte Änderungen",
        }
    }

    pub fn unsaved_switching(&self) -> &'static str {
        match self.lang {
            Language::English => "switching snippets",
            Language::German => "Snippets wechseln",
        }
    }

    pub fn unsaved_creating(&self) -> &'static str {
        match self.lang {
            Language::English => "creating a snippet",
            Language::German => "ein Snippet erstellen",
        }
    }

    pub fn unsaved_duplicating(&self) -> &'static str {
        match self.lang {
            Language::English => "duplicating a snippet",
            Language::German => "ein Snippet duplizieren",
        }
    }

    pub fn unsaved_deleting(&self) -> &'static str {
        match self.lang {
            Language::English => "deleting a snippet",
            Language::German => "ein Snippet löschen",
        }
    }

    pub fn unsaved_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "reloading the configuration",
            Language::German => "die Konfiguration neu laden",
        }
    }

    pub fn unsaved_undoing(&self) -> &'static str {
        match self.lang {
            Language::English => "undoing the last change",
            Language::German => "die letzte Änderung rückgängig machen",
        }
    }

    pub fn unsaved_closing(&self) -> &'static str {
        match self.lang {
            Language::English => "closing WayExpand",
            Language::German => "WayExpand schließen",
        }
    }

    pub fn save_before(&self, action: &str) -> String {
        match self.lang {
            Language::English => format!("Save changes before {}?", action),
            Language::German => format!("Änderungen vor {} speichern?", action),
        }
    }

    pub fn save_continue(&self) -> &'static str {
        match self.lang {
            Language::English => "Save and continue",
            Language::German => "Speichern und fortfahren",
        }
    }

    pub fn discard(&self) -> &'static str {
        match self.lang {
            Language::English => "Discard",
            Language::German => "Verwerfen",
        }
    }

    pub fn delete_confirm(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Delete this snippet? This cannot be recovered except through Undo."
            }
            Language::German => {
                "Dieses Snippet löschen? Dies kann nur über Rückgängig wiederhergestellt werden."
            }
        }
    }

    pub fn delete_button(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete snippet",
            Language::German => "Snippet löschen",
        }
    }
}
