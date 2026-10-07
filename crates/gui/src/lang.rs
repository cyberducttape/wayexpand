mod status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    pub fn from_env() -> Self {
        std::env::var("LANG")
            .ok()
            .and_then(|lang| {
                if lang.starts_with("de") {
                    Some(Language::German)
                } else {
                    None
                }
            })
            .unwrap_or(Language::English)
    }

    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::English),
            "de" => Some(Language::German),
            _ => None,
        }
    }
}

pub struct Strings {
    lang: Language,
}

impl Strings {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }

    pub fn set_language(&mut self, lang: Language) {
        self.lang = lang;
    }
}

mod appearance;
mod diagnostics;
mod dialogs;
mod dialogs_editor;
mod editor;
mod editor_chrome;
mod import;
mod sidebar;
mod status_messages;
mod toolbar;
