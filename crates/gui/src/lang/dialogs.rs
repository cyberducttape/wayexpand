use super::{Language, Strings};

impl Strings {
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
            (Language::English, "window_tracker") => "Window tracker (KWin or GNOME Shell)",
            (Language::Indonesian, "window_tracker") => "Pelacak jendela (KWin atau GNOME Shell)",
            (Language::German, "window_tracker") => "Fensterverfolgung (KWin oder GNOME Shell)",
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

    pub fn buffer_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Characters retained while looking for a trigger (1–4096).",
            Language::Indonesian => "Jumlah karakter yang disimpan saat mencari pemicu (1–4096).",
            Language::German => {
                "Zeichen, die bei der Suche nach einem Auslöser beibehalten werden (1–4096)."
            }
        }
    }

    pub fn undo_chord_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Only works when the active route verifies the target document. Current IBus and daemon routes cannot verify their output, so this shortcut cannot undo it. Leave empty to disable."
            }
            Language::Indonesian => {
                "Hanya berfungsi jika rute aktif memverifikasi dokumen target. Rute IBus dan daemon saat ini tidak dapat memverifikasi keluarannya, jadi pintasan ini tidak dapat membatalkannya. Kosongkan untuk menonaktifkan."
            }
            Language::German => {
                "Funktioniert nur, wenn die aktive Route das Zieldokument verifiziert. Die aktuellen IBus- und Daemon-Routen können ihre Ausgabe nicht verifizieren, daher kann diese Tastenkombination sie nicht rückgängig machen. Leer lassen zum Deaktivieren."
            }
        }
    }
}
