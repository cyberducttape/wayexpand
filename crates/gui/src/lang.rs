mod status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
    Indonesian,
}

impl Language {
    pub fn from_env() -> Self {
        std::env::var("LANG")
            .ok()
            .and_then(|lang| {
                if lang.starts_with("de") {
                    Some(Language::German)
                } else if lang.starts_with("id") {
                    Some(Language::Indonesian)
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
            Language::Indonesian => "id",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::English),
            "de" => Some(Language::German),
            "id" => Some(Language::Indonesian),
            _ => None,
        }
    }
}

pub struct Strings {
    lang: Language,
}

pub(crate) struct RouteTrustDetails<'a> {
    pub route: &'a str,
    pub maturity: &'a str,
    pub sensitive_fields: Option<bool>,
    pub trigger_precondition: &'a str,
    pub output_transaction: &'a str,
    pub application_result: &'a str,
    pub certified: bool,
}

impl Strings {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }

    pub fn set_language(&mut self, lang: Language) {
        self.lang = lang;
    }
}

/// One `&'static str` method per entry: English, German, Indonesian.
#[macro_export]
macro_rules! texts {
    ($($name:ident => $en:expr, $de:expr, $id:expr;)*) => {
        impl $crate::lang::Strings {
            $(
                pub fn $name(&self) -> &'static str {
                    match self.lang {
                        $crate::lang::Language::English => $en,
                        $crate::lang::Language::German => $de,
                        $crate::lang::Language::Indonesian => $id,
                    }
                }
            )*
        }
    };
}

/// Fill each `{}` or `{name}` placeholder of a translated template with the
/// next value, in order. Translations keep the placeholders' order.
pub(crate) fn fill(template: &str, values: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut values = values.iter();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        match values.next() {
            Some(value) => out.push_str(&value.to_string()),
            None => out.push_str(&rest[start..start + end + 1]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

mod appearance;
mod diagnostics;
mod diagnostics_ui;
mod dialogs;
mod dialogs_editor;
mod editor;
mod editor_chrome;
mod import;
mod insertion;
mod onboarding;
mod sidebar;
mod status_messages;
mod toolbar;
pub use toolbar::HeaderState;
