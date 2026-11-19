use super::{Language, Strings};

impl Strings {
    pub fn undo_button(&self, count: usize) -> String {
        match self.lang {
            // The history depth is useful once there is something to undo;
            // "Undo (0)" only advertised an action that does nothing.
            Language::English if count == 0 => "Undo".to_owned(),
            Language::German if count == 0 => "Rückgängig".to_owned(),
            Language::English => format!("Undo ({count})"),
            Language::Indonesian => format!("Urungkan ({count})"),
            Language::German => format!("Rückgängig ({count})"),
        }
    }

    pub fn no_matches_filter(&self, filter: &str) -> String {
        match self.lang {
            Language::English => format!("No matches for \"{}\".", filter),
            Language::Indonesian => format!("Tidak ada yang cocok untuk \"{}\".", filter),
            Language::German => format!("Keine Treffer für \"{}\".", filter),
        }
    }

    pub fn no_matches_category(&self, filter: &str, category: &str) -> String {
        match self.lang {
            Language::English => {
                format!("No matches for \"{}\" in {}.", filter, category)
            }
            Language::Indonesian => {
                format!("Tidak ada yang cocok untuk \"{}\" di {}.", filter, category)
            }
            Language::German => format!("Keine Treffer für \"{}\" in {}.", filter, category),
        }
    }

    pub fn no_snippets_category(&self, category: &str) -> String {
        match self.lang {
            Language::English => format!("No snippets in {}.", category),
            Language::Indonesian => format!("Tidak ada cuplikan di {}.", category),
            Language::German => format!("Keine Snippets in {}.", category),
        }
    }
}
