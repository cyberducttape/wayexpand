use super::{Language, Strings};

impl Strings {
    pub fn ready(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Ready",
            Language::German => "Fertig",
        }
    }

    pub fn new_snippet_draft(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "New snippet draft · complete it and save to add it"
            }
            Language::German => "Neuer Snippet-Entwurf · ausfüllen und speichern zum Hinzufügen",
        }
    }

    pub fn new_snippet_replacement_required(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "A new snippet needs replacement text before it can be saved"
            }
            Language::German => {
                "Ein neues Snippet benötigt Ersatztext, bevor es gespeichert werden kann"
            }
        }
    }

    pub fn no_selection(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "No snippet selected",
            Language::German => "Kein Snippet ausgewählt",
        }
    }
}
