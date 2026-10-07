use super::{Language, Strings};

impl Strings {
    // Status line
    pub fn status_diagnostics_refreshed(&self) -> &'static str {
        match self.lang {
            Language::English => "Diagnostics refreshed",
            Language::German => "Diagnose aktualisiert",
        }
    }

    pub fn diagnostics_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running diagnostics…",
            Language::German => "Diagnose läuft…",
        }
    }

    pub fn daemon_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "Reloading daemon configuration…",
            Language::German => "Daemon-Konfiguration wird neu geladen…",
        }
    }

    pub fn config_reload_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Loading configuration…",
            Language::German => "Konfiguration wird geladen…",
        }
    }

    pub fn status_reload_discarded_due_edits(&self) -> &'static str {
        match self.lang {
            Language::English => "Reload discarded because the configuration or draft changed while loading",
            Language::German => "Neuladen verworfen, da sich Konfiguration oder Entwurf während des Ladens geändert hat",
        }
    }

    pub fn daemon_control_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Updating daemon state…",
            Language::German => "Daemon-Status wird aktualisiert…",
        }
    }

    pub fn background_queue_full(&self) -> &'static str {
        match self.lang {
            Language::English => "Background task queue is busy; try again shortly",
            Language::German => {
                "Hintergrundwarteschlange ausgelastet; bitte gleich erneut versuchen"
            }
        }
    }

    pub fn background_runtime_stopped(&self) -> &'static str {
        match self.lang {
            Language::English => "Background runtime stopped unexpectedly",
            Language::German => "Hintergrunddienst wurde unerwartet beendet",
        }
    }

    pub fn status_daemon_not_reloaded(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("daemon did not reload: {detail}"),
            Language::German => format!("Daemon hat nicht neu geladen: {detail}"),
        }
    }

    pub fn status_buffer_limit_not_a_number(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("buffer limit must be a whole number ({detail})"),
            Language::German => format!("Puffergrenze muss eine ganze Zahl sein ({detail})"),
        }
    }

    pub fn status_settings_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings invalid: {detail}"),
            Language::German => format!("Einstellungen ungültig: {detail}"),
        }
    }

    pub fn status_settings_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings rejected: {detail}"),
            Language::German => format!("Einstellungen abgelehnt: {detail}"),
        }
    }

    pub fn status_settings_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Settings saved atomically",
            Language::German => "Einstellungen atomar gespeichert",
        }
    }

    pub fn status_appearance_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Appearance preference could not be saved: {detail}"),
            Language::German => {
                format!("Darstellungseinstellung konnte nicht gespeichert werden: {detail}")
            }
        }
    }

    pub fn status_settings_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings save failed: {detail}"),
            Language::German => format!("Speichern der Einstellungen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_font_size_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Font size saved",
            Language::German => "Schriftgröße gespeichert",
        }
    }

    pub fn status_import_loaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Espanso library loaded for review",
            Language::German => "Espanso-Bibliothek zur Prüfung geladen",
        }
    }

    pub fn status_import_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import failed: {detail}"),
            Language::German => format!("Import fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_import_needs_clean_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "Save or discard the current draft before importing",
            Language::German => "Aktuellen Entwurf vor dem Import speichern oder verwerfen",
        }
    }

    pub fn status_import_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import rejected: {detail}"),
            Language::German => format!("Import abgelehnt: {detail}"),
        }
    }

    pub fn status_import_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import save failed: {detail}"),
            Language::German => format!("Speichern des Imports fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_imported_with_report(
        &self,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Espanso import applied: {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Espanso-Import angewendet: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn status_config_reloaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration reloaded",
            Language::German => "Konfiguration neu geladen",
        }
    }

    pub fn status_config_changed_externally(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration changed outside WayExpand; reload it before saving",
            Language::German => {
                "Konfiguration wurde außerhalb von WayExpand geändert; vor dem Speichern neu laden"
            }
        }
    }

    pub fn status_reload_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Reload failed: {detail}"),
            Language::German => format!("Neu laden fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command settings invalid: {detail}"),
            Language::German => format!("Befehlseinstellungen ungültig: {detail}"),
        }
    }

    pub fn status_save_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save rejected: {detail}"),
            Language::German => format!("Speichern abgelehnt: {detail}"),
        }
    }

    pub fn status_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save failed: {detail}"),
            Language::German => format!("Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_saving(&self) -> &'static str {
        match self.lang {
            Language::English => "Saving configuration…",
            Language::German => "Konfiguration wird gespeichert…",
        }
    }

    pub fn status_save_busy(&self) -> &'static str {
        match self.lang {
            Language::English => "A save is already in progress",
            Language::German => "Ein Speichervorgang läuft bereits",
        }
    }

    pub fn status_save_completed_with_newer_edits(&self) -> &'static str {
        match self.lang {
            Language::English => "Save completed; newer edits remain unsaved",
            Language::German => "Gespeichert; neuere Änderungen sind noch nicht gespeichert",
        }
    }

    pub fn status_snippet_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet saved atomically",
            Language::German => "Snippet atomar gespeichert",
        }
    }

    pub fn status_nothing_to_undo(&self) -> &'static str {
        match self.lang {
            Language::English => "Nothing to undo",
            Language::German => "Nichts zum Rückgängigmachen",
        }
    }

    pub fn status_undo_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Undo save failed: {detail}"),
            Language::German => format!("Rückgängig-Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_undone(&self) -> &'static str {
        match self.lang {
            Language::English => "Undid the last saved change",
            Language::German => "Letzte gespeicherte Änderung rückgängig gemacht",
        }
    }

    pub fn status_created(&self) -> &'static str {
        match self.lang {
            Language::English => "Created a new snippet",
            Language::German => "Neues Snippet erstellt",
        }
    }

    pub fn status_create_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Create failed: {detail}"),
            Language::German => format!("Erstellen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_duplicated(&self) -> &'static str {
        match self.lang {
            Language::English => "Duplicated snippet",
            Language::German => "Snippet dupliziert",
        }
    }

    pub fn status_duplicate_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Duplicate failed: {detail}"),
            Language::German => format!("Duplizieren fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_deleted(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Deleted {trigger}"),
            Language::German => format!("{trigger} gelöscht"),
        }
    }

    pub fn status_delete_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Delete failed: {detail}"),
            Language::German => format!("Löschen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_snippet_enabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} enabled"),
            Language::German => format!("{trigger} aktiviert"),
        }
    }

    pub fn status_snippet_disabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} disabled"),
            Language::German => format!("{trigger} deaktiviert"),
        }
    }

    pub fn status_toggle_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Toggle failed: {detail}"),
            Language::German => format!("Umschalten fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_paused(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion paused",
            Language::German => "Erweiterung angehalten",
        }
    }

    pub fn status_resumed(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion resumed",
            Language::German => "Erweiterung fortgesetzt",
        }
    }

    pub fn status_control_unavailable(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Daemon control unavailable: {detail}"),
            Language::German => format!("Daemon-Steuerung nicht verfügbar: {detail}"),
        }
    }

    pub fn status_app_filter_added(&self, value: &str) -> String {
        match self.lang {
            Language::English => format!("Added \"{value}\" to the app filter"),
            Language::German => format!("\"{value}\" zum App-Filter hinzugefügt"),
        }
    }

    pub fn status_window_unidentified(&self) -> &'static str {
        match self.lang {
            Language::English => "Could not identify the focused window",
            Language::German => "Fokussiertes Fenster konnte nicht ermittelt werden",
        }
    }

    pub fn status_no_focused_window(&self) -> &'static str {
        match self.lang {
            Language::English => "No focused window to detect (focus is on the desktop)",
            Language::German => {
                "Kein fokussiertes Fenster erkennbar (Fokus liegt auf der Arbeitsfläche)"
            }
        }
    }

    pub fn status_detection_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection is unavailable here (KDE Plasma only for now)",
            Language::German => {
                "Fenstererkennung ist hier nicht verfügbar (derzeit nur KDE Plasma)"
            }
        }
    }

    pub fn status_detection_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection failed unexpectedly",
            Language::German => "Fenstererkennung ist unerwartet fehlgeschlagen",
        }
    }

    pub fn status_preview_copied(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview copied to clipboard",
            Language::German => "Vorschau in die Zwischenablage kopiert",
        }
    }

    pub fn status_enable_command_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable the dynamic command first",
            Language::German => "Zuerst den dynamischen Befehl aktivieren",
        }
    }

    pub fn status_command_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command failed: {detail}"),
            Language::German => format!("Befehl fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_preview_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Command preview failed unexpectedly",
            Language::German => "Befehlsvorschau ist unerwartet fehlgeschlagen",
        }
    }
}
