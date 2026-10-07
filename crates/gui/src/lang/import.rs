use super::{Language, Strings};

impl Strings {
    pub fn import_preview_summary(
        &self,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Migration preview: {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Importvorschau: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn search_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Search triggers, descriptions, tags, and categories (Ctrl+F)",
            Language::German => {
                "Trigger, Beschreibungen, Schlagwörter und Kategorien durchsuchen (Strg+F)"
            }
        }
    }

    pub fn command_backed_summary(&self) -> &'static str {
        match self.lang {
            Language::English => "Command-backed snippet",
            Language::German => "Befehlsgestütztes Snippet",
        }
    }

    pub fn app_filter_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Leave empty for every app. Add an app to limit this snippet to that application.",
            Language::German => "Leer lassen für alle Apps. Eine App hinzufügen, um dieses Snippet auf diese Anwendung zu begrenzen.",
        }
    }

    pub fn app_filter_advanced_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Advanced syntax: bare values or app_id_exact:<id> match one normalized desktop app ID. app_id_glob:<pattern> and title_contains:<text> intentionally use weaker matching. Filters currently require the KWin window tracker.",
            Language::German => "Erweiterte Syntax: Einzelwerte oder app_id_exact:<id> gleichen eine normalisierte Desktop-App-ID exakt ab. app_id_glob:<Muster> und title_contains:<Text> verwenden absichtlich schwächeres Matching. Filter benötigen derzeit den KWin-Fenster-Tracker.",
        }
    }

    pub fn app_filter_advanced(&self) -> &'static str {
        match self.lang {
            Language::English => "Advanced matching syntax",
            Language::German => "Erweiterte Matching-Syntax",
        }
    }

    pub fn matching(&self) -> &'static str {
        match self.lang {
            Language::English => "Matching",
            Language::German => "Erkennung",
        }
    }

    pub fn aliases(&self) -> &'static str {
        match self.lang {
            Language::English => "Aliases",
            Language::German => "Aliase",
        }
    }

    pub fn add_alias(&self) -> &'static str {
        match self.lang {
            Language::English => "Add alias…",
            Language::German => "Alias hinzufügen…",
        }
    }

    pub fn remove_alias(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove alias",
            Language::German => "Alias entfernen",
        }
    }

    pub fn add_tag(&self) -> &'static str {
        match self.lang {
            Language::English => "Add tag…",
            Language::German => "Tag hinzufügen…",
        }
    }

    pub fn remove_tag(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove tag",
            Language::German => "Tag entfernen",
        }
    }

    pub fn add_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Add app id…",
            Language::German => "App-ID hinzufügen…",
        }
    }

    pub fn remove_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove app",
            Language::German => "App entfernen",
        }
    }

    pub fn add_argument(&self) -> &'static str {
        match self.lang {
            Language::English => "+ Add argument",
            Language::German => "+ Argument hinzufügen",
        }
    }

    pub fn move_up(&self) -> &'static str {
        match self.lang {
            Language::English => "Move up",
            Language::German => "Nach oben",
        }
    }

    pub fn move_down(&self) -> &'static str {
        match self.lang {
            Language::English => "Move down",
            Language::German => "Nach unten",
        }
    }

    pub fn remove_argument(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove argument",
            Language::German => "Argument entfernen",
        }
    }

    pub fn environment(&self) -> &'static str {
        match self.lang {
            Language::English => "Environment",
            Language::German => "Umgebung",
        }
    }

    pub fn environment_minimal(&self) -> &'static str {
        match self.lang {
            Language::English => "Minimal",
            Language::German => "Minimal",
        }
    }

    pub fn environment_inherit(&self) -> &'static str {
        match self.lang {
            Language::English => "Inherit",
            Language::German => "Übernehmen",
        }
    }

    pub fn environment_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Minimal passes only a safe PATH and the variables listed below; Inherit passes the editor's whole environment.",
            Language::German => "Minimal übergibt nur einen sicheren PATH und die unten aufgeführten Variablen; Übernehmen übergibt die gesamte Umgebung des Editors.",
        }
    }

    pub fn pass_environment(&self) -> &'static str {
        match self.lang {
            Language::English => "Pass environment variables",
            Language::German => "Umgebungsvariablen übergeben",
        }
    }

    pub fn add_environment_variable(&self) -> &'static str {
        match self.lang {
            Language::English => "+ Add environment variable",
            Language::German => "+ Umgebungsvariable hinzufügen",
        }
    }

    pub fn remove_environment_variable(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove environment variable",
            Language::German => "Umgebungsvariable entfernen",
        }
    }

    pub fn no_changes_to_save(&self) -> &'static str {
        match self.lang {
            Language::English => "No unsaved changes",
            Language::German => "Keine ungespeicherten Änderungen",
        }
    }

    pub fn duplicate_needs_selection(&self) -> &'static str {
        match self.lang {
            Language::English => "Select a snippet to duplicate it",
            Language::German => "Wählen Sie ein Snippet zum Duplizieren",
        }
    }

    pub fn template_variable_description(&self, variable: &str) -> &'static str {
        match (self.lang, variable) {
            (Language::English, "{{date}}") => "UTC date",
            (Language::German, "{{date}}") => "Datum (UTC)",
            (Language::English, "{{time}}") => "UTC time",
            (Language::German, "{{time}}") => "Uhrzeit (UTC)",
            (Language::English, "{{datetime}}") => "UTC date and time",
            (Language::German, "{{datetime}}") => "Datum und Uhrzeit (UTC)",
            (Language::English, "{{date+1d}}") => {
                "Tomorrow's date (also -1d, +1w; date, time or datetime; d/w/h/m units)"
            }
            (Language::German, "{{date+1d}}") => {
                "Morgiges Datum (auch -1d, +1w; date, time oder datetime; Einheiten d/w/h/m)"
            }
            (Language::English, "{{cursor}}") => {
                "Place the cursor here after expanding (libei and wlroots backends)"
            }
            (Language::German, "{{cursor}}") => {
                "Setzt den Cursor nach dem Erweitern hierher (Backends libei und wlroots)"
            }
            (Language::English, "{{username}}") => "Current user",
            (Language::German, "{{username}}") => "Aktueller Benutzer",
            (Language::English, "{{hostname}}") => "Local hostname",
            (Language::German, "{{hostname}}") => "Lokaler Rechnername",
            (Language::English, "{{unix_timestamp}}") => "Unix timestamp",
            (Language::German, "{{unix_timestamp}}") => "Unix-Zeitstempel",
            (Language::English, "{{newline}}") => "Line break",
            (Language::German, "{{newline}}") => "Zeilenumbruch",
            (Language::English, "{{tab}}") => "Tab character",
            (Language::German, "{{tab}}") => "Tabulatorzeichen",
            _ => "",
        }
    }

    pub fn description_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "What is this snippet for? (shown in the list)",
            Language::German => "Wofür ist dieses Snippet? (in der Liste angezeigt)",
        }
    }

    pub fn command_not_run_by_gui(&self) -> &'static str {
        match self.lang {
            Language::English => "This command is not run by the GUI.",
            Language::German => "Dieser Befehl wird nicht von der GUI ausgeführt.",
        }
    }

    pub fn unknown_desktop(&self) -> &'static str {
        match self.lang {
            Language::English => "Linux desktop",
            Language::German => "Linux-Desktop",
        }
    }

    pub fn try_live_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Matcher preview",
            Language::German => "Matcher-Vorschau",
        }
    }

    pub fn try_live_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Tests matching only — not desktop integration. It does not test capture, focus, portals, sensitive fields, or insertion; command snippets are not run.",
            Language::German => "Testet nur Matching — keine Desktop-Integration. Erfassung, Fokus, Portale, sensible Felder und Einfügen werden nicht getestet; Befehls-Snippets laufen nicht.",
        }
    }

    pub fn try_live_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Type a trigger and watch it expand…",
            Language::German => "Tippen Sie einen Auslöser und sehen Sie zu…",
        }
    }

    pub fn try_live_stats(&self, expansions: usize, saved: usize) -> String {
        match self.lang {
            Language::English => format!(
                "{expansions} expansion{} · {saved} keystrokes saved",
                if expansions == 1 { "" } else { "s" }
            ),
            Language::German => format!(
                "{expansions} Erweiterung{} · {saved} Tastenanschläge gespart",
                if expansions == 1 { "" } else { "en" }
            ),
        }
    }

    pub fn clear(&self) -> &'static str {
        match self.lang {
            Language::English => "Clear",
            Language::German => "Leeren",
        }
    }

    pub fn picker_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Search snippets…",
            Language::German => "Snippets suchen…",
        }
    }

    pub fn picker_no_results(&self) -> &'static str {
        match self.lang {
            Language::English => "No matching snippets",
            Language::German => "Keine passenden Snippets",
        }
    }

    pub fn picker_footer_daemon(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "↑↓ select · Enter types it into the app you were using · Esc cancels"
            }
            Language::German => {
                "↑↓ auswählen · Enter tippt es in die vorherige App · Esc bricht ab"
            }
        }
    }

    pub fn picker_footer_clipboard(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Can't type into the previous app (no daemon or window tracking) · Enter copies the snippet to the clipboard"
            }
            Language::German => {
                "Tippen in die vorherige App nicht möglich (kein Dienst oder Fenster-Tracking) · Enter kopiert das Snippet in die Zwischenablage"
            }
        }
    }

    pub fn picker_footer_identity_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Exact window identity: unavailable · Quick Picker typing: clipboard-only"
            }
            Language::German => {
                "Exakte Fensteridentität: nicht verfügbar · Quick Picker-Eingabe: nur Zwischenablage"
            }
        }
    }

    pub fn picker_copied(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Copied {trigger} · paste it with Ctrl+V, then press Esc"),
            Language::German => {
                format!("{trigger} kopiert · mit Strg+V einfügen, dann Esc drücken")
            }
        }
    }

    pub fn picker_inserting(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Inserting {trigger}…"),
            Language::German => format!("{trigger} wird eingefügt …"),
        }
    }

    pub fn picker_waiting_for_focus(&self) -> &'static str {
        match self.lang {
            Language::English => "Waiting for the original window to regain focus safely.",
            Language::German => {
                "Warten, bis das ursprüngliche Fenster sicher wieder fokussiert ist."
            }
        }
    }

    pub fn picker_insert_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Could not safely return to the original window.",
            Language::German => {
                "Das ursprüngliche Fenster konnte nicht sicher wieder fokussiert werden."
            }
        }
    }

    pub fn picker_copy_instead(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy instead",
            Language::German => "Stattdessen kopieren",
        }
    }

    pub fn cancel(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancel",
            Language::German => "Abbrechen",
        }
    }

    pub fn turn_on(&self) -> &'static str {
        match self.lang {
            Language::English => "Turn on WayExpand",
            Language::German => "WayExpand einschalten",
        }
    }

    pub fn turning_on(&self) -> &'static str {
        match self.lang {
            Language::English => "Turning on…",
            Language::German => "Wird eingeschaltet…",
        }
    }

    pub fn turn_on_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Runs `wayexpand setup --yes`: enables the safest detected input path for this desktop. It never grants raw keyboard access; your desktop may ask for permission.",
            Language::German => "Führt `wayexpand setup --yes` aus: aktiviert den sichersten erkannten Eingabeweg. Es wird nie roher Tastaturzugriff gewährt; der Desktop fragt eventuell nach Erlaubnis.",
        }
    }

    pub fn status_setup_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Turning on WayExpand…",
            Language::German => "WayExpand wird eingeschaltet…",
        }
    }

    pub fn status_setup_cancelling(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancelling setup…",
            Language::German => "Einrichtung wird abgebrochen…",
        }
    }

    pub fn status_setup_done(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand is on · try a normal text field in an app you use; see Desktop details for route limits",
            Language::German => "WayExpand ist aktiv · probieren Sie ein normales Textfeld in einer verwendeten App; siehe Desktop-Details für Einschränkungen",
        }
    }

    pub fn status_setup_failed(&self, detail: &str) -> String {
        match (self.lang, detail.is_empty()) {
            (Language::English, true) => "Setup failed; run `wayexpand doctor` for details".into(),
            (Language::German, true) => {
                "Einrichtung fehlgeschlagen; Details mit `wayexpand doctor`".into()
            }
            (Language::English, false) => format!("Setup failed: {detail}"),
            (Language::German, false) => format!("Einrichtung fehlgeschlagen: {detail}"),
        }
    }

    pub fn onboarding_turn_on_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Turn on WayExpand",
            Language::German => "WayExpand einschalten",
        }
    }

    pub fn onboarding_turn_on_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Enables the safest input path WayExpand detects on this desktop. Your desktop may ask for permission.",
            Language::German => "Aktiviert den sichersten Eingabeweg, den WayExpand auf diesem Desktop erkennt. Der Desktop fragt eventuell nach Erlaubnis.",
        }
    }

    pub fn onboarding_snippet_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Add your first snippet",
            Language::German => "Erstes Snippet anlegen",
        }
    }

    pub fn onboarding_snippet_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Start with a test snippet, write your own, or bring your Espanso library.",
            Language::German => "Beginnen Sie mit einem Test-Snippet, schreiben Sie ein eigenes oder übernehmen Sie Ihre Espanso-Bibliothek.",
        }
    }

    pub fn onboarding_details_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Desktop details",
            Language::German => "Desktop-Details",
        }
    }
}
