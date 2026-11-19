use super::{Language, Strings};

impl Strings {
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
}
