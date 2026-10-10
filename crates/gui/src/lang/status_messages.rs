use super::{Language, Strings};

impl Strings {
    pub fn ready(&self) -> &'static str {
        match self.lang {
            Language::English => "Ready",
            Language::Indonesian => "Siap",
            Language::German => "Fertig",
        }
    }

    pub fn new_snippet_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "New snippet draft · complete it and save to add it",
            Language::Indonesian => "Draf cuplikan baru · lengkapi dan simpan untuk menambahkannya",
            Language::German => "Neuer Snippet-Entwurf · ausfüllen und speichern zum Hinzufügen",
        }
    }

    pub fn new_snippet_replacement_required(&self) -> &'static str {
        match self.lang {
            Language::English => "A new snippet needs replacement text before it can be saved",
            Language::Indonesian => {
                "Cuplikan baru memerlukan teks pengganti sebelum dapat disimpan"
            }
            Language::German => {
                "Ein neues Snippet benötigt Ersatztext, bevor es gespeichert werden kann"
            }
        }
    }

    pub fn no_selection(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippet selected",
            Language::Indonesian => "Tidak ada cuplikan yang dipilih",
            Language::German => "Kein Snippet ausgewählt",
        }
    }
}
