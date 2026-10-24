use super::{Language, Strings};

impl Strings {
    pub fn diagnostics_title(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand diagnostics",
            Language::Indonesian => "Diagnostik WayExpand",
            Language::German => "WayExpand Diagnose",
        }
    }

    pub fn runtime_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Runtime health",
            Language::Indonesian => "Kesehatan runtime",
            Language::German => "Laufzeit-Zustand",
        }
    }

    pub fn compatibility_center(&self) -> &'static str {
        match self.lang {
            Language::English => "Compatibility Center",
            Language::Indonesian => "Pusat Kompatibilitas",
            Language::German => "Kompatibilitätszentrum",
        }
    }

    pub fn production_readiness(&self) -> &'static str {
        match self.lang {
            Language::English => "Production readiness",
            Language::Indonesian => "Kesiapan produksi",
            Language::German => "Produktionsreife",
        }
    }

    pub fn production_readiness_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Unavailable · no compatible live route",
            Language::Indonesian => "Tidak tersedia · tidak ada rute aktif yang kompatibel",
            Language::German => "Nicht verfügbar · kein kompatibler aktiver Weg",
        }
    }

    pub fn production_readiness_limited(&self) -> &'static str {
        match self.lang {
            Language::English => "Limited · one or more safety guarantees are missing",
            Language::Indonesian => "Terbatas · satu atau lebih jaminan keamanan tidak tersedia",
            Language::German => "Eingeschränkt · eine oder mehrere Sicherheitsgarantien fehlen",
        }
    }

    pub fn production_readiness_pending(&self) -> &'static str {
        match self.lang {
            Language::English => "Certification pending · live compositor/client evidence required",
            Language::Indonesian => {
                "Sertifikasi tertunda · diperlukan bukti langsung dari compositor/klien"
            }
            Language::German => {
                "Zertifizierung ausstehend · Nachweise mit echtem Compositor/Client erforderlich"
            }
        }
    }

    pub fn run_compatibility_test(&self) -> &'static str {
        match self.lang {
            Language::English => "Run protocol checks",
            Language::Indonesian => "Jalankan pemeriksaan protokol",
            Language::German => "Protokollprüfungen ausführen",
        }
    }

    pub fn live_test_boundary(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "These checks verify protocol and capability availability, not live typing in another application. Use a temporary, non-sensitive text field to verify the selected route before relying on it."
            }
            Language::Indonesian => {
                "Pemeriksaan ini memverifikasi ketersediaan protokol dan kemampuan, bukan pengetikan langsung di aplikasi lain. Gunakan kolom teks sementara yang tidak sensitif untuk memverifikasi rute yang dipilih sebelum mengandalkannya."
            }
            Language::German => {
                "Diese Prüfungen verifizieren Protokoll- und Fähigkeitsverfügbarkeit, nicht das Live-Tippen in einer anderen Anwendung. Prüfe den ausgewählten Weg vor dem Einsatz in einem temporären, nicht sensiblen Textfeld."
            }
        }
    }

    pub fn sensitive_field_warning(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Sensitive-field protection is unavailable on this route. Do not use it for passwords or other secrets."
            }
            Language::Indonesian => {
                "Perlindungan kolom sensitif tidak tersedia pada rute ini. Jangan gunakan untuk kata sandi atau rahasia lainnya."
            }
            Language::German => {
                "Der Schutz sensibler Felder ist auf diesem Weg nicht verfügbar. Nicht für Passwörter oder andere Geheimnisse verwenden."
            }
        }
    }

    pub fn uncertain_output_warning(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Output is not guaranteed atomic. A timeout or backend failure can leave an uncertain result; avoid retrying into a different focused window until the field is checked."
            }
            Language::Indonesian => {
                "Keluaran tidak dijamin atomik. Batas waktu atau kegagalan backend dapat meninggalkan hasil yang tidak pasti; jangan mencoba ulang ke jendela lain yang sedang fokus sebelum kolomnya diperiksa."
            }
            Language::German => {
                "Die Ausgabe ist nicht garantiert atomar. Ein Timeout oder Backendfehler kann ein ungewisses Ergebnis hinterlassen; nicht in ein anderes fokussiertes Fenster wiederholen, bevor das Feld geprüft wurde."
            }
        }
    }

    pub fn active_route(&self) -> &'static str {
        match self.lang {
            Language::English => "Active route",
            Language::Indonesian => "Rute aktif",
            Language::German => "Aktiver Weg",
        }
    }

    pub fn configuration_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration",
            Language::Indonesian => "Konfigurasi",
            Language::German => "Konfiguration",
        }
    }

    pub fn configuration_healthy(&self) -> &'static str {
        match self.lang {
            Language::English => "Healthy",
            Language::Indonesian => "Sehat",
            Language::German => "In Ordnung",
        }
    }

    pub fn configuration_invalid(&self) -> &'static str {
        match self.lang {
            Language::English => "Invalid",
            Language::Indonesian => "Tidak valid",
            Language::German => "Ungültig",
        }
    }

    pub fn safety(&self) -> &'static str {
        match self.lang {
            Language::English => "Safety",
            Language::Indonesian => "Keamanan",
            Language::German => "Sicherheit",
        }
    }

    pub fn capture_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English => "Keyboard capture",
            Language::Indonesian => "Tangkapan papan ketik",
            Language::German => "Tastaturerfassung",
        }
    }

    pub fn injection_guarantees(&self) -> &'static str {
        match self.lang {
            Language::English => "Text output",
            Language::Indonesian => "Keluaran teks",
            Language::German => "Textausgabe",
        }
    }

    pub fn output_mode(&self) -> &'static str {
        match self.lang {
            Language::English => "Negotiated output mode",
            Language::Indonesian => "Mode keluaran yang dinegosiasikan",
            Language::German => "Ausgehandelter Ausgabemodus",
        }
    }

    pub fn keysym_fallback_warning(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Keysym fallback is active: output is layout-dependent and paced, so longer replacements may be noticeably slower."
            }
            Language::Indonesian => {
                "Fallback keysym aktif: keluaran bergantung pada tata letak dan diketik bertahap, sehingga teks pengganti yang panjang bisa terasa lebih lambat."
            }
            Language::German => {
                "Keysym-Fallback ist aktiv: Die Ausgabe hängt vom Layout ab und wird gebremst; längere Ersetzungen können spürbar langsamer sein."
            }
        }
    }

    pub fn latency(&self) -> &'static str {
        match self.lang {
            Language::English => "Latency (recent samples)",
            Language::Indonesian => "Latensi (sampel terbaru)",
            Language::German => "Latenz (letzte Messwerte)",
        }
    }

    pub fn matcher_latency(&self) -> &'static str {
        match self.lang {
            Language::English => "Matcher latency",
            Language::Indonesian => "Latensi pencocokan",
            Language::German => "Matcher-Latenz",
        }
    }

    pub fn output_latency(&self) -> &'static str {
        match self.lang {
            Language::English => "Backend injection latency",
            Language::Indonesian => "Latensi injeksi backend",
            Language::German => "Latenz der Backend-Injektion",
        }
    }

    pub fn no_samples(&self) -> &'static str {
        match self.lang {
            Language::English => "no samples yet",
            Language::Indonesian => "belum ada sampel",
            Language::German => "noch keine Messwerte",
        }
    }

    pub fn application_context(&self) -> &'static str {
        match self.lang {
            Language::English => "Application context",
            Language::Indonesian => "Konteks aplikasi",
            Language::German => "Anwendungskontext",
        }
    }

    pub fn capability_label(&self, key: &str) -> &'static str {
        match (self.lang, key) {
            (Language::English, "sensitive_focus") => "Sensitive-field awareness",
            (Language::Indonesian, "sensitive_focus") => "Kesadaran kolom sensitif",
            (Language::German, "sensitive_focus") => "Erkennung sensibler Felder",
            (Language::English, "exclusive") => "Exclusive keyboard capture",
            (Language::Indonesian, "exclusive") => "Tangkapan papan ketik eksklusif",
            (Language::German, "exclusive") => "Exklusive Tastaturerfassung",
            (Language::English, "reliable_key_state") => "Key press/release tracking",
            (Language::Indonesian, "reliable_key_state") => "Pelacakan tekan/lepas tombol",
            (Language::German, "reliable_key_state") => "Tastenanschlag/-loslassen verfolgen",
            (Language::English, "capture_passthrough") => "Unsupported-key forwarding",
            (Language::Indonesian, "capture_passthrough") => "Penerusan tombol yang tidak didukung",
            (Language::German, "capture_passthrough") => "Weitergabe nicht unterstützter Tasten",
            (Language::English, "composition") => "IME composition awareness",
            (Language::Indonesian, "composition") => "Kesadaran komposisi IME",
            (Language::German, "composition") => "IME-Kompositionserkennung",
            (Language::English, "local_compose") => "Dead-key/Compose sequence tracking",
            (Language::Indonesian, "local_compose") => "Pelacakan urutan dead-key/Compose",
            (Language::German, "local_compose") => "Tot-/Compose-Tastenfolgen verfolgen",
            (Language::English, "layout") => "Runtime keyboard-layout awareness",
            (Language::Indonesian, "layout") => "Kesadaran tata letak papan ketik saat runtime",
            (Language::German, "layout") => "Erkennung wechselnder Tastaturlayouts",
            (Language::English, "window_tracker") => "KWin window tracker",
            (Language::Indonesian, "window_tracker") => "Pelacak jendela KWin",
            (Language::German, "window_tracker") => "KWin-Fensterverfolgung",
            (Language::English, "exact_window_identity") => "Exact window identity",
            (Language::Indonesian, "exact_window_identity") => "Identitas jendela yang tepat",
            (Language::German, "exact_window_identity") => "Exakte Fensteridentität",
            (Language::English, "atomic_replace") => "Atomic text replacement",
            (Language::Indonesian, "atomic_replace") => "Penggantian teks atomik",
            (Language::German, "atomic_replace") => "Atomarer Textersatz",
            (Language::English, "unicode") => "Layout-independent Unicode",
            (Language::Indonesian, "unicode") => "Unicode yang tidak bergantung pada tata letak",
            (Language::German, "unicode") => "Tastaturlayout-unabhängiges Unicode",
            (Language::English, "cursor") => "Cursor repositioning",
            (Language::Indonesian, "cursor") => "Pemosisian ulang kursor",
            (Language::German, "cursor") => "Cursorpositionierung",
            (Language::English, "injection_passthrough") => "Key press/release forwarding",
            (Language::Indonesian, "injection_passthrough") => "Penerusan tekan/lepas tombol",
            (Language::German, "injection_passthrough") => "Weitergabe von Tastenanschlägen",
            _ => "",
        }
    }

    pub fn capability_available(&self) -> &'static str {
        match self.lang {
            Language::English => "Available",
            Language::Indonesian => "Tersedia",
            Language::German => "Verfügbar",
        }
    }

    pub fn capability_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Not provided",
            Language::Indonesian => "Tidak disediakan",
            Language::German => "Nicht verfügbar",
        }
    }

    pub fn capability_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "Unknown · update/restart daemon",
            Language::Indonesian => "Tidak diketahui · perbarui/mulai ulang daemon",
            Language::German => "Unbekannt · Dienst aktualisieren/neu starten",
        }
    }

    pub fn daemon(&self) -> &'static str {
        match self.lang {
            Language::English => "Daemon",
            Language::Indonesian => "Daemon",
            Language::German => "Daemon",
        }
    }

    pub fn backends(&self) -> &'static str {
        match self.lang {
            Language::English => "Backends",
            Language::Indonesian => "Backend",
            Language::German => "Backends",
        }
    }

    pub fn refresh(&self) -> &'static str {
        match self.lang {
            Language::English => "Refresh",
            Language::Indonesian => "Segarkan",
            Language::German => "Aktualisieren",
        }
    }

    pub fn protocol_probes(&self) -> &'static str {
        match self.lang {
            Language::English => "Non-mutating protocol probes",
            Language::Indonesian => "Pemeriksaan protokol tanpa perubahan",
            Language::German => "Nicht-mutierende Protokoll-Tests",
        }
    }

    pub fn import_dialog_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Import Espanso library",
            Language::Indonesian => "Impor pustaka Espanso",
            Language::German => "Espanso-Bibliothek importieren",
        }
    }

    pub fn source_yaml(&self) -> &'static str {
        match self.lang {
            Language::English => "Source YAML file",
            Language::Indonesian => "Berkas YAML sumber",
            Language::German => "YAML-Quelldatei",
        }
    }

    pub fn import_preview_info(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview first, then merge while preserving current snippets or explicitly replace the library.",
            Language::Indonesian => "Pratinjau dahulu, lalu gabungkan sambil mempertahankan cuplikan saat ini atau ganti pustaka secara eksplisit.",
            Language::German => "Zuerst prüfen, dann aktuelle Snippets beim Zusammenführen bewahren oder die Bibliothek ausdrücklich ersetzen.",
        }
    }

    pub fn load_preview(&self) -> &'static str {
        match self.lang {
            Language::English => "Load preview",
            Language::Indonesian => "Muat pratinjau",
            Language::German => "Vorschau laden",
        }
    }

    pub fn picker_cancel(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancel",
            Language::Indonesian => "Batal",
            Language::German => "Abbrechen",
        }
    }

    pub fn replace_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Replace current library",
            Language::Indonesian => "Ganti pustaka saat ini",
            Language::German => "Aktuelle Bibliothek ersetzen",
        }
    }

    pub fn merge_library(&self) -> &'static str {
        match self.lang {
            Language::English => "Merge (keep current conflicts)",
            Language::Indonesian => "Gabungkan (pertahankan konflik saat ini)",
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
            Language::English => format!(
                "Import merged: {added} added, {duplicates} identical duplicates ignored, {conflicts} trigger conflicts kept; {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::Indonesian => format!(
                "Impor digabungkan: {added} ditambahkan, {duplicates} duplikat identik diabaikan, {conflicts} konflik pemicu dipertahankan; {fully_migrated} termigrasi penuh, {with_warnings} dengan peringatan, {unsupported} tidak didukung"
            ),
            Language::German => format!(
                "Import zusammengeführt: {added} hinzugefügt, {duplicates} identische Duplikate ignoriert, {conflicts} Trigger-Konflikte beibehalten; {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn settings_title(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand settings",
            Language::Indonesian => "Pengaturan WayExpand",
            Language::German => "WayExpand-Einstellungen",
        }
    }

    pub fn buffer_limit(&self) -> &'static str {
        match self.lang {
            Language::English => "Matcher buffer limit",
            Language::Indonesian => "Batas buffer pencocok",
            Language::German => "Matcher-Puffer-Limit",
        }
    }

    pub fn buffer_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Characters retained while looking for a trigger (1–4096).",
            Language::Indonesian => "Jumlah karakter yang disimpan saat mencari pemicu (1–4096).",
            Language::German => {
                "Zeichen, die bei der Suche nach einem Auslöser beibehalten werden (1–4096)."
            }
        }
    }

    pub fn undo_chord(&self) -> &'static str {
        match self.lang {
            Language::English => "Undo shortcut",
            Language::Indonesian => "Pintasan urungkan",
            Language::German => "Rückgängig-Tastenkombination",
        }
    }

    pub fn undo_chord_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Pressed right after an expansion with nothing typed in between, reverts it. Leave empty to disable."
            }
            Language::Indonesian => {
                "Ditekan tepat setelah ekspansi tanpa ada ketikan di antaranya, akan membatalkannya. Kosongkan untuk menonaktifkan."
            }
            Language::German => {
                "Direkt nach einer Erweiterung gedrückt (ohne dazwischen zu tippen), macht sie rückgängig. Leer lassen zum Deaktivieren."
            }
        }
    }

    pub fn save_settings(&self) -> &'static str {
        match self.lang {
            Language::English => "Save settings",
            Language::Indonesian => "Simpan pengaturan",
            Language::German => "Einstellungen speichern",
        }
    }

    pub fn close(&self) -> &'static str {
        match self.lang {
            Language::English => "Close",
            Language::Indonesian => "Tutup",
            Language::German => "Schließen",
        }
    }
}
