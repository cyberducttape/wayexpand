use super::{Language, Strings};

impl Strings {
    pub fn snippet_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet details",
            Language::German => "Snippet-Details",
        }
    }

    pub fn trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Trigger",
            Language::German => "Auslöser",
        }
    }

    pub fn trigger_hint(&self) -> &'static str {
        match self.lang {
            Language::English => ";;hello",
            Language::German => ";;hallo",
        }
    }

    pub fn duplicate_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: another snippet already uses this trigger; saving will be rejected.",
            Language::German => "Warnung: Ein anderes Snippet verwendet bereits diesen Auslöser; das Speichern wird abgelehnt.",
        }
    }

    pub fn trigger_tip(&self) -> &'static str {
        match self.lang {
            Language::English => "Tip: use a distinctive prefix such as ;; or : to avoid accidental matches.",
            Language::German => "Tipp: Verwenden Sie ein eindeutiges Präfix wie ;; oder :, um versehentliche Treffer zu vermeiden.",
        }
    }

    pub fn description(&self) -> &'static str {
        match self.lang {
            Language::English => "Description",
            Language::German => "Beschreibung",
        }
    }

    pub fn tags(&self) -> &'static str {
        match self.lang {
            Language::English => "Tags",
            Language::German => "Tags",
        }
    }

    pub fn category(&self) -> &'static str {
        match self.lang {
            Language::English => "Category",
            Language::German => "Kategorie",
        }
    }

    pub fn category_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "productivity, shortcuts, custom",
            Language::German => "Produktivität, Verknüpfungen, Benutzerdefiniert",
        }
    }

    pub fn existing(&self) -> &'static str {
        match self.lang {
            Language::English => "Choose…",
            Language::German => "Auswählen…",
        }
    }

    pub fn app_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "Only in these apps",
            Language::German => "Nur in diesen Apps",
        }
    }

    pub fn detect_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Use current app",
            Language::German => "Aktuelle App verwenden",
        }
    }

    pub fn detect_app_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Detect the app you were last focused on before switching to WayExpand (KDE Plasma only for now)",
            Language::German => "Erkennen Sie die App, auf die Sie sich zuletzt konzentriert haben, bevor Sie zu WayExpand wechseln (nur KDE Plasma)",
        }
    }

    pub fn window_tracking_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: if window tracking isn't available on your compositor, this snippet will never match rather than matching everywhere.",
            Language::German => "Warnung: Wenn die Fenster-Verfolgung in Ihrem Kompositor nicht verfügbar ist, passt dieses Snippet nie, anstatt überall zu passen.",
        }
    }

    pub fn enabled(&self) -> &'static str {
        match self.lang {
            Language::English => "Enabled",
            Language::German => "Aktiviert",
        }
    }

    pub fn immediate(&self) -> &'static str {
        match self.lang {
            Language::English => "Immediate",
            Language::German => "Sofort",
        }
    }

    pub fn word_boundary(&self) -> &'static str {
        match self.lang {
            Language::English => "Word boundary",
            Language::German => "Wortgrenze",
        }
    }

    pub fn propagate_case(&self) -> &'static str {
        match self.lang {
            Language::English => "Follow trigger capitalization",
            Language::German => "Groß-/Kleinschreibung anpassen",
        }
    }

    pub fn propagate_case_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Typing the trigger in UPPERCASE or Capitalized form applies the same casing to the replacement"
            }
            Language::German => {
                "Wird der Auslöser in GROSSBUCHSTABEN oder Großschreibung eingegeben, erhält der Ersatztext dieselbe Schreibweise"
            }
        }
    }

    pub fn replacement(&self) -> &'static str {
        match self.lang {
            Language::English => "Replacement",
            Language::German => "Ersatz",
        }
    }

    pub fn insertion_limit_warning(&self, count: usize, limit: usize, mode: &str) -> String {
        match self.lang {
            Language::English => format!(
                "This snippet has {count} characters, but the active {mode} mode supports at most {limit}. It may be refused before insertion."
            ),
            Language::German => format!(
                "Dieses Snippet hat {count} Zeichen, aber der aktive Modus {mode} unterstützt höchstens {limit}. Die Einfügung wird möglicherweise abgelehnt."
            ),
        }
    }

    pub fn save_changes(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes",
            Language::German => "Änderungen speichern",
        }
    }

    pub fn save_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes (Ctrl+S)",
            Language::German => "Änderungen speichern (Strg+S)",
        }
    }

    pub fn delete(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete…",
            Language::German => "Löschen…",
        }
    }

    pub fn preview(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview",
            Language::German => "Vorschau",
        }
    }

    pub fn input(&self) -> &'static str {
        match self.lang {
            Language::English => "Input",
            Language::German => "Eingabe",
        }
    }

    pub fn input_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "text containing the trigger",
            Language::German => "Text mit dem Auslöser",
        }
    }

    pub fn use_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Use trigger",
            Language::German => "Auslöser verwenden",
        }
    }

    pub fn copy(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy",
            Language::German => "Kopieren",
        }
    }

    pub fn copy_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy the previewed output",
            Language::German => "Kopieren Sie die vorbereitete Ausgabe",
        }
    }

    pub fn template_variables(&self) -> &'static str {
        match self.lang {
            Language::English => "Template variables",
            Language::German => "Template-Variablen",
        }
    }

    pub fn template_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert a safe built-in value into the replacement.",
            Language::German => "Fügen Sie einen sicheren integrierten Wert in den Ersatz ein.",
        }
    }

    pub fn dynamic_command(&self) -> &'static str {
        match self.lang {
            Language::English => "Dynamic command (optional)",
            Language::German => "Dynamischer Befehl (optional)",
        }
    }

    pub fn command_checkbox(&self) -> &'static str {
        match self.lang {
            Language::English => "Run a direct program when this snippet matches",
            Language::German => "Führen Sie ein direktes Programm aus, wenn dieses Snippet passt",
        }
    }

    pub fn command_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Only the configured executable is run; shell syntax is never interpreted. Arguments are entered one per line.",
            Language::German => "Nur die konfigurierte ausführbare Datei wird ausgeführt; Shell-Syntax wird nie interpretiert. Argumente werden eine pro Zeile eingegeben.",
        }
    }

    pub fn command_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: this runs a local executable when the trigger matches.",
            Language::German => {
                "Warnung: Dieses Programm wird ausgeführt, wenn der Auslöser passt."
            }
        }
    }

    pub fn program(&self) -> &'static str {
        match self.lang {
            Language::English => "Program",
            Language::German => "Programm",
        }
    }

    pub fn program_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "uname",
            Language::German => "uname",
        }
    }

    pub fn timeout_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Timeout ms",
            Language::German => "Zeitüberschreitung ms",
        }
    }

    pub fn cache_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Cache ms",
            Language::German => "Cache ms",
        }
    }

    pub fn arguments(&self) -> &'static str {
        match self.lang {
            Language::English => "Arguments",
            Language::German => "Argumente",
        }
    }

    pub fn command_backed_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "The typed text comes from the command below. This replacement is kept but not used while the command is enabled, including when the command fails."
            }
            Language::German => {
                "Der eingefügte Text stammt vom Befehl unten. Dieser Ersatztext bleibt erhalten, wird aber nicht verwendet, solange der Befehl aktiv ist – auch nicht, wenn der Befehl fehlschlägt."
            }
        }
    }
}
