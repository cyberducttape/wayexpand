use super::{Language, Strings};

impl Strings {
    pub fn preview_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview app",
            Language::German => "Vorschau-App",
        }
    }

    pub fn preview_app_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "leave empty for no focused app",
            Language::German => "leer lassen für keine fokussierte App",
        }
    }

    pub fn run_once(&self) -> &'static str {
        match self.lang {
            Language::English => "Run locally (advanced)",
            Language::German => "Lokal ausführen (erweitert)",
        }
    }

    pub fn run_through_broker(&self) -> &'static str {
        match self.lang {
            Language::English => "Run through broker",
            Language::German => "Über Broker ausführen",
        }
    }

    pub fn running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running…",
            Language::German => "Läuft…",
        }
    }

    pub fn managed_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This managed action runs through the configured Action Broker, using its policy, timeout, and environment restrictions. Run it once to see the current output."
            }
            Language::German => {
                "Diese verwaltete Aktion läuft über den konfigurierten Action Broker mit dessen Richtlinien, Zeitlimit und Umgebungsbeschränkungen. Einmal ausführen, um die aktuelle Ausgabe zu sehen."
            }
        }
    }

    pub fn direct_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This direct program runs locally in the GUI process with desktop-user privileges. It does not reproduce the daemon service sandbox; use it only as an advanced preview."
            }
            Language::German => {
                "Dieses direkte Programm läuft lokal im GUI-Prozess mit den Rechten des Desktop-Benutzers. Die Sandbox des Daemons wird nicht reproduziert; verwenden Sie dies nur als erweiterte Vorschau."
            }
        }
    }

    pub fn detecting_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Detecting focused application…",
            Language::German => "Fokussierte Anwendung wird erkannt…",
        }
    }

    pub fn stopping_app_detection(&self) -> &'static str {
        match self.lang {
            Language::English => "Stopping application detection…",
            Language::German => "Anwendungserkennung wird beendet…",
        }
    }

    pub fn no_description(&self) -> &'static str {
        match self.lang {
            Language::English => "No description",
            Language::German => "Keine Beschreibung",
        }
    }

    pub fn click_to_enable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to enable",
            Language::German => "Zum Aktivieren klicken",
        }
    }

    pub fn click_to_disable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to disable",
            Language::German => "Zum Deaktivieren klicken",
        }
    }

    pub fn selection_stale(&self) -> &'static str {
        match self.lang {
            Language::English => "Selection is out of date; choose a snippet again.",
            Language::German => "Auswahl ist veraltet; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn draft_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet draft unavailable; choose a snippet again.",
            Language::German => "Snippet-Entwurf nicht verfügbar; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn no_snippets_match_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippets match this filter.",
            Language::German => "Keine Snippets entsprechen diesem Filter.",
        }
    }

    pub fn configuration_file(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration",
            Language::German => "Konfiguration",
        }
    }
}
