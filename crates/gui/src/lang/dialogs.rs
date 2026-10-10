use super::{Language, Strings};

impl Strings {
    pub fn diagnostics_title(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "WayExpand diagnostics",
            Language::German => "WayExpand Diagnose",
        }
    }

    pub fn runtime_health(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Runtime health",
            Language::German => "Laufzeit-Zustand",
        }
    }

    pub fn compatibility_center(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Compatibility Center",
            Language::German => "Kompatibilitätszentrum",
        }
    }

    pub fn production_readiness(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Production readiness",
            Language::German => "Produktionsreife",
        }
    }

    pub fn production_readiness_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Unavailable · no compatible live route",
            Language::German => "Nicht verfügbar · kein kompatibler aktiver Weg",
        }
    }

    pub fn production_readiness_limited(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Limited · one or more safety guarantees are missing"
            }
            Language::German => "Eingeschränkt · eine oder mehrere Sicherheitsgarantien fehlen",
        }
    }

    pub fn production_readiness_pending(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Certification pending · live compositor/client evidence required"
            }
            Language::German => {
                "Zertifizierung ausstehend · Nachweise mit echtem Compositor/Client erforderlich"
            }
        }
    }

    pub fn run_compatibility_test(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Run protocol checks",
            Language::German => "Protokollprüfungen ausführen",
        }
    }

    pub fn live_test_boundary(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "These checks verify protocol and capability availability, not live typing in another application. Use a temporary, non-sensitive text field to verify the selected route before relying on it."
            }
            Language::German => {
                "Diese Prüfungen verifizieren Protokoll- und Fähigkeitsverfügbarkeit, nicht das Live-Tippen in einer anderen Anwendung. Prüfe den ausgewählten Weg vor dem Einsatz in einem temporären, nicht sensiblen Textfeld."
            }
        }
    }

    pub fn sensitive_field_warning(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Sensitive-field protection is unavailable on this route. Do not use it for passwords or other secrets."
            }
            Language::German => {
                "Der Schutz sensibler Felder ist auf diesem Weg nicht verfügbar. Nicht für Passwörter oder andere Geheimnisse verwenden."
            }
        }
    }

    pub fn uncertain_output_warning(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Output is not guaranteed atomic. A timeout or backend failure can leave an uncertain result; avoid retrying into a different focused window until the field is checked."
            }
            Language::German => {
                "Die Ausgabe ist nicht garantiert atomar. Ein Timeout oder Backendfehler kann ein ungewisses Ergebnis hinterlassen; nicht in ein anderes fokussiertes Fenster wiederholen, bevor das Feld geprüft wurde."
            }
        }
    }

    pub fn active_route(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Active route",
            Language::German => "Aktiver Weg",
        }
    }

    pub fn configuration_health(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Configuration",
            Language::German => "Konfiguration",
        }
    }

    pub fn configuration_healthy(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Healthy",
            Language::German => "In Ordnung",
        }
    }

    pub fn configuration_invalid(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Invalid",
            Language::German => "Ungültig",
        }
    }

    pub fn safety(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Safety",
            Language::German => "Sicherheit",
        }
    }

    pub fn capture_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Keyboard capture",
            Language::German => "Tastaturerfassung",
        }
    }

    pub fn injection_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Text output",
            Language::German => "Textausgabe",
        }
    }

    pub fn output_mode(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Negotiated output mode",
            Language::German => "Ausgehandelter Ausgabemodus",
        }
    }

    pub fn keysym_fallback_warning(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Keysym fallback is active: output is layout-dependent and paced, so longer replacements may be noticeably slower."
            }
            Language::German => {
                "Keysym-Fallback ist aktiv: Die Ausgabe hängt vom Layout ab und wird gebremst; längere Ersetzungen können spürbar langsamer sein."
            }
        }
    }

    pub fn latency(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Latency (recent samples)",
            Language::German => "Latenz (letzte Messwerte)",
        }
    }

    pub fn matcher_latency(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Matcher / preparation",
            Language::German => "Matcher / Vorbereitung",
        }
    }

    pub fn output_latency(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Output backend / injection",
            Language::German => "Ausgabebackend / Einfügen",
        }
    }

    pub fn no_samples(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "no samples yet",
            Language::German => "noch keine Messwerte",
        }
    }

    pub fn application_context(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Application context",
            Language::German => "Anwendungskontext",
        }
    }

    pub fn capability_label(&self, key: &str) -> &'static str {
        match (self.lang, key) {
            (Language::English | Language::Indonesian, "sensitive_focus") => {
                "Sensitive-field awareness"
            }
            (Language::German, "sensitive_focus") => "Erkennung sensibler Felder",
            (Language::English | Language::Indonesian, "exclusive") => "Exclusive keyboard capture",
            (Language::German, "exclusive") => "Exklusive Tastaturerfassung",
            (Language::English | Language::Indonesian, "reliable_key_state") => {
                "Key press/release tracking"
            }
            (Language::German, "reliable_key_state") => "Tastenanschlag/-loslassen verfolgen",
            (Language::English | Language::Indonesian, "capture_passthrough") => {
                "Unsupported-key forwarding"
            }
            (Language::German, "capture_passthrough") => "Weitergabe nicht unterstützter Tasten",
            (Language::English | Language::Indonesian, "composition") => {
                "IME composition awareness"
            }
            (Language::German, "composition") => "IME-Kompositionserkennung",
            (Language::English | Language::Indonesian, "local_compose") => {
                "Dead-key/Compose sequence tracking"
            }
            (Language::German, "local_compose") => "Tot-/Compose-Tastenfolgen verfolgen",
            (Language::English | Language::Indonesian, "layout") => {
                "Runtime keyboard-layout awareness"
            }
            (Language::German, "layout") => "Erkennung wechselnder Tastaturlayouts",
            (Language::English | Language::Indonesian, "window_tracker") => "KWin window tracker",
            (Language::German, "window_tracker") => "KWin-Fensterverfolgung",
            (Language::English | Language::Indonesian, "exact_window_identity") => {
                "Exact window identity"
            }
            (Language::German, "exact_window_identity") => "Exakte Fensteridentität",
            (Language::English | Language::Indonesian, "atomic_replace") => {
                "Atomic text replacement"
            }
            (Language::German, "atomic_replace") => "Atomarer Textersatz",
            (Language::English | Language::Indonesian, "unicode") => "Layout-independent Unicode",
            (Language::German, "unicode") => "Tastaturlayout-unabhängiges Unicode",
            (Language::English | Language::Indonesian, "cursor") => "Cursor repositioning",
            (Language::German, "cursor") => "Cursorpositionierung",
            (Language::English | Language::Indonesian, "injection_passthrough") => {
                "Key press/release forwarding"
            }
            (Language::German, "injection_passthrough") => "Weitergabe von Tastenanschlägen",
            _ => "",
        }
    }

    pub fn capability_available(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Available",
            Language::German => "Verfügbar",
        }
    }

    pub fn capability_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Not provided",
            Language::German => "Nicht verfügbar",
        }
    }

    pub fn capability_unknown(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Unknown · update/restart daemon",
            Language::German => "Unbekannt · Dienst aktualisieren/neu starten",
        }
    }

    pub fn daemon(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Daemon",
            Language::German => "Daemon",
        }
    }

    pub fn backends(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Backends",
            Language::German => "Backends",
        }
    }

    pub fn refresh(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Refresh",
            Language::German => "Aktualisieren",
        }
    }

    pub fn protocol_probes(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Non-mutating protocol probes",
            Language::German => "Nicht-mutierende Protokoll-Tests",
        }
    }

    pub fn import_dialog_title(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Import Espanso library",
            Language::German => "Espanso-Bibliothek importieren",
        }
    }

    pub fn source_yaml(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Source YAML file",
            Language::German => "YAML-Quelldatei",
        }
    }

    pub fn import_preview_info(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Preview first, then merge while preserving current snippets or explicitly replace the library.",
            Language::German => "Zuerst prüfen, dann aktuelle Snippets beim Zusammenführen bewahren oder die Bibliothek ausdrücklich ersetzen.",
        }
    }

    pub fn load_preview(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Load preview",
            Language::German => "Vorschau laden",
        }
    }

    pub fn picker_cancel(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Cancel",
            Language::German => "Abbrechen",
        }
    }

    pub fn replace_library(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Replace current library",
            Language::German => "Aktuelle Bibliothek ersetzen",
        }
    }

    pub fn merge_library(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Merge (keep current conflicts)",
            Language::German => "Zusammenführen (Konflikte behalten)",
        }
    }

    pub fn status_import_merged(
        &self,
        added: usize,
        duplicates: usize,
        conflicts: usize,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English | Language::Indonesian => format!(
                "Import merged: {added} added, {duplicates} identical duplicates ignored, {conflicts} trigger conflicts kept; {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::German => format!(
                "Import zusammengeführt: {added} hinzugefügt, {duplicates} identische Duplikate ignoriert, {conflicts} Trigger-Konflikte beibehalten; {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn settings_title(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "WayExpand settings",
            Language::German => "WayExpand-Einstellungen",
        }
    }

    pub fn buffer_limit(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Matcher buffer limit",
            Language::German => "Matcher-Puffer-Limit",
        }
    }

    pub fn buffer_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Characters retained while looking for a trigger (1–4096)."
            }
            Language::German => {
                "Zeichen, die bei der Suche nach einem Auslöser beibehalten werden (1–4096)."
            }
        }
    }

    pub fn undo_chord(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Undo shortcut",
            Language::German => "Rückgängig-Tastenkombination",
        }
    }

    pub fn undo_chord_help(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => {
                "Pressed right after an expansion with nothing typed in between, reverts it. Leave empty to disable."
            }
            Language::German => {
                "Direkt nach einer Erweiterung gedrückt (ohne dazwischen zu tippen), macht sie rückgängig. Leer lassen zum Deaktivieren."
            }
        }
    }

    pub fn save_settings(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Save settings",
            Language::German => "Einstellungen speichern",
        }
    }

    pub fn close(&self) -> &'static str {
        match self.lang {
            Language::English | Language::Indonesian => "Close",
            Language::German => "Schließen",
        }
    }
}
