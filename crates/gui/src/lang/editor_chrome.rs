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
                "This named action runs through the configured Action Broker and is subject to its allowlist and execution policy. Broker auditing is optional and health-dependent; check wayexpand doctor --json for audit health."
            }
            Language::German => {
                "Diese benannte Aktion läuft über den konfigurierten Action Broker und unterliegt dessen Zulassungsliste und Ausführungsrichtlinien. Das Broker-Audit ist optional und vom Zustand des Protokolls abhängig; den Audit-Status zeigt wayexpand doctor --json."
            }
        }
    }

    pub fn direct_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This direct program runs locally with desktop-user privileges, outside the daemon and broker sandboxes. It is not recorded in the Action Broker audit; use only for trusted commands."
            }
            Language::German => {
                "Dieses Programm läuft lokal mit den Rechten des Desktop-Benutzers außerhalb der Daemon- und Broker-Sandbox. Es wird nicht im Action-Broker-Audit protokolliert; nur für vertrauenswürdige Befehle verwenden."
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
