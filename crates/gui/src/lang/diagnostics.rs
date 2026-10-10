use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn fleet_layers(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Fleet layers",
            Language::German => "Flotten-Ebenen",
        }
    }

    pub fn not_checked(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Not checked",
            Language::German => "Nicht geprüft",
        }
    }

    /// A readable label for a backend's state. The raw
    /// `implementation/availability/permission` triple is still shown
    /// verbatim next to it so the GUI stays a faithful mirror of
    /// `wayexpand doctor` rather than paraphrasing it away.
    pub fn backend_state(&self, state: BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English | Language::Indonesian, BackendState::Available) => "Ready",
            (Language::English | Language::Indonesian, BackendState::RequiresPermission) => {
                "Needs permission"
            }
            (Language::English | Language::Indonesian, BackendState::Implemented) => "Not probed",
            (Language::English | Language::Indonesian, BackendState::Unavailable) => "Unavailable",
            (Language::English | Language::Indonesian, BackendState::NotImplemented) => {
                "Not implemented"
            }
            (Language::German, BackendState::Available) => "Bereit",
            (Language::German, BackendState::RequiresPermission) => "Berechtigung nötig",
            (Language::German, BackendState::Implemented) => "Nicht geprüft",
            (Language::German, BackendState::Unavailable) => "Nicht verfügbar",
            (Language::German, BackendState::NotImplemented) => "Nicht implementiert",
        }
    }
}
