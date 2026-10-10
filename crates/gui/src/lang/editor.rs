use super::{Language, Strings};

impl Strings {
    pub fn snippet_details(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Snippet details",
            Language::German => "Snippet-Details",
        }
    }

    pub fn trigger(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Trigger",
            Language::German => "Auslöser",
        }
    }

    pub fn trigger_hint(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => ";;hello",
            Language::German => ";;hallo",
        }
    }

    pub fn duplicate_trigger(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Warning: another snippet already uses this trigger; saving will be rejected.",
            Language::German => "Warnung: Ein anderes Snippet verwendet bereits diesen Auslöser; das Speichern wird abgelehnt.",
        }
    }

    pub fn trigger_tip(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Tip: use a distinctive prefix such as ;; or : to avoid accidental matches.",
            Language::German => "Tipp: Verwenden Sie ein eindeutiges Präfix wie ;; oder :, um versehentliche Treffer zu vermeiden.",
        }
    }

    pub fn description(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Description",
            Language::German => "Beschreibung",
        }
    }

    pub fn tags(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Tags",
            Language::German => "Tags",
        }
    }

    pub fn category(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Category",
            Language::German => "Kategorie",
        }
    }

    pub fn category_hint(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "productivity, shortcuts, custom",
            Language::German => "Produktivität, Verknüpfungen, Benutzerdefiniert",
        }
    }

    pub fn existing(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Choose…",
            Language::German => "Auswählen…",
        }
    }

    pub fn app_filter(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Only in these apps",
            Language::German => "Nur in diesen Apps",
        }
    }

    pub fn detect_app(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Use current app",
            Language::German => "Aktuelle App verwenden",
        }
    }

    pub fn detect_app_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Detect the app you were last focused on before switching to WayExpand (KDE Plasma only for now)",
            Language::German => "Erkennen Sie die App, auf die Sie sich zuletzt konzentriert haben, bevor Sie zu WayExpand wechseln (nur KDE Plasma)",
        }
    }

    pub fn window_tracking_warning(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Warning: if window tracking isn't available on your compositor, this snippet will never match rather than matching everywhere.",
            Language::German => "Warnung: Wenn die Fenster-Verfolgung in Ihrem Kompositor nicht verfügbar ist, passt dieses Snippet nie, anstatt überall zu passen.",
        }
    }

    pub fn enabled(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Enabled",
            Language::German => "Aktiviert",
        }
    }

    pub fn immediate(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Immediate",
            Language::German => "Sofort",
        }
    }

    pub fn word_boundary(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Word boundary",
            Language::German => "Wortgrenze",
        }
    }

    pub fn propagate_case(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Follow trigger capitalization",
            Language::German => "Groß-/Kleinschreibung anpassen",
        }
    }

    pub fn propagate_case_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Typing the trigger in UPPERCASE or Capitalized form applies the same casing to the replacement"
            }
            Language::German => {
                "Wird der Auslöser in GROSSBUCHSTABEN oder Großschreibung eingegeben, erhält der Ersatztext dieselbe Schreibweise"
            }
        }
    }

    pub fn replacement(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Replacement",
            Language::German => "Ersatz",
        }
    }

    pub fn insertion_limit_warning(&self, count: usize, limit: usize, mode: &str) -> String {
        match self.lang {
            Language::English | Language::Indonesian => format!(
                "This snippet has {count} characters, but the active {mode} mode supports at most {limit}. It may be refused before insertion."
            ),
            Language::German => format!(
                "Dieses Snippet hat {count} Zeichen, aber der aktive Modus {mode} unterstützt höchstens {limit}. Die Einfügung wird möglicherweise abgelehnt."
            ),
        }
    }

    pub fn save_changes(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Save changes",
            Language::German => "Änderungen speichern",
        }
    }

    pub fn save_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Save changes (Ctrl+S)",
            Language::German => "Änderungen speichern (Strg+S)",
        }
    }

    pub fn delete(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Delete…",
            Language::German => "Löschen…",
        }
    }

    pub fn preview(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Preview",
            Language::German => "Vorschau",
        }
    }

    pub fn input(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Input",
            Language::German => "Eingabe",
        }
    }

    pub fn input_hint(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "text containing the trigger",
            Language::German => "Text mit dem Auslöser",
        }
    }

    pub fn use_trigger(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Use trigger",
            Language::German => "Auslöser verwenden",
        }
    }

    pub fn copy(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Copy",
            Language::German => "Kopieren",
        }
    }

    pub fn copy_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Copy the previewed output",
            Language::German => "Kopieren Sie die vorbereitete Ausgabe",
        }
    }

    pub fn template_variables(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Template variables",
            Language::German => "Template-Variablen",
        }
    }

    pub fn template_help(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Insert a safe built-in value into the replacement."
            }
            Language::German => "Fügen Sie einen sicheren integrierten Wert in den Ersatz ein.",
        }
    }

    pub fn dynamic_command(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Dynamic command (optional)",
            Language::German => "Dynamischer Befehl (optional)",
        }
    }

    pub fn execution_type(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Execution type",
            Language::German => "Ausführungsart",
        }
    }

    pub fn direct_executable(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Direct executable",
            Language::German => "Direktes Programm",
        }
    }

    pub fn managed_action(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Managed action",
            Language::German => "Verwaltete Aktion",
        }
    }

    pub fn action(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Action",
            Language::German => "Aktion",
        }
    }

    pub fn select_or_type_action(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Select or type an action",
            Language::German => "Aktion auswählen oder eingeben",
        }
    }

    pub fn action_hint(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "cluster-status",
            Language::German => "cluster-status",
        }
    }

    pub fn command_checkbox(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Run a direct program when this snippet matches"
            }
            Language::German => "Führen Sie ein direktes Programm aus, wenn dieses Snippet passt",
        }
    }

    pub fn command_help(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Only the configured executable is run; shell syntax is never interpreted. Arguments are entered one per line.",
            Language::German => "Nur die konfigurierte ausführbare Datei wird ausgeführt; Shell-Syntax wird nie interpretiert. Argumente werden eine pro Zeile eingegeben.",
        }
    }

    pub fn command_warning(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Warning: this runs a local executable when the trigger matches."
            }
            Language::German => {
                "Warnung: Dieses Programm wird ausgeführt, wenn der Auslöser passt."
            }
        }
    }

    pub fn program(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Program",
            Language::German => "Programm",
        }
    }

    pub fn program_hint(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "uname",
            Language::German => "uname",
        }
    }

    pub fn timeout_ms(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Timeout ms",
            Language::German => "Zeitüberschreitung ms",
        }
    }

    pub fn cache_ms(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Cache ms",
            Language::German => "Cache ms",
        }
    }

    pub fn arguments(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Arguments",
            Language::German => "Argumente",
        }
    }

    pub fn command_backed_help(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "The typed text comes from the command below. This replacement is kept but not used while the command is enabled, including when the command fails."
            }
            Language::German => {
                "Der eingefügte Text stammt vom Befehl unten. Dieser Ersatztext bleibt erhalten, wird aber nicht verwendet, solange der Befehl aktiv ist – auch nicht, wenn der Befehl fehlschlägt."
            }
        }
    }
}
