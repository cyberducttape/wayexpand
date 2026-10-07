use super::{Language, Strings};

impl Strings {
    pub fn your_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Your reusable text library",
            Language::German => "Ihre wiederverwendbare Textbibliothek",
        }
    }

    pub fn filtered_snippets(&self) -> &'static str {
        match self.lang {
            Language::English => "Filtered snippets",
            Language::German => "Gefilterte Snippets",
        }
    }

    pub fn new_button(&self) -> &'static str {
        match self.lang {
            Language::English => "+ New",
            Language::German => "+ Neu",
        }
    }

    pub fn new_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Create a new snippet (Ctrl+N)",
            Language::German => "Ein neues Snippet erstellen (Strg+N)",
        }
    }

    pub fn duplicate(&self) -> &'static str {
        match self.lang {
            Language::English => "Duplicate",
            Language::German => "Duplizieren",
        }
    }

    pub fn undo_button(&self, count: usize) -> String {
        match self.lang {
            // The history depth is useful once there is something to undo;
            // "Undo (0)" only advertised an action that does nothing.
            Language::English if count == 0 => "Undo".to_owned(),
            Language::German if count == 0 => "Rückgängig".to_owned(),
            Language::English => format!("Undo ({count})"),
            Language::German => format!("Rückgängig ({count})"),
        }
    }

    pub fn all(&self) -> &'static str {
        match self.lang {
            Language::English => "All",
            Language::German => "Alle",
        }
    }

    pub fn no_snippets(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippets yet.",
            Language::German => "Noch keine Snippets.",
        }
    }

    pub fn no_matches_filter(&self, filter: &str) -> String {
        match self.lang {
            Language::English => format!("No matches for \"{}\".", filter),
            Language::German => format!("Keine Treffer für \"{}\".", filter),
        }
    }

    pub fn no_matches_category(&self, filter: &str, category: &str) -> String {
        match self.lang {
            Language::English => format!("No matches for \"{}\" in {}.", filter, category),
            Language::German => format!("Keine Treffer für \"{}\" in {}.", filter, category),
        }
    }

    pub fn no_snippets_category(&self, category: &str) -> String {
        match self.lang {
            Language::English => format!("No snippets in {}.", category),
            Language::German => format!("Keine Snippets in {}.", category),
        }
    }

    pub fn clear_filters(&self) -> &'static str {
        match self.lang {
            Language::English => "Clear filters",
            Language::German => "Filter löschen",
        }
    }
}
