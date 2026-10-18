use super::{Language, Strings};

impl Strings {
    // Status line
    pub fn status_diagnostics_refreshed(&self) -> &'static str {
        match self.lang {
            Language::English => "Diagnostics refreshed",
            Language::Indonesian => "Diagnostik diperbarui",
            Language::German => "Diagnose aktualisiert",
        }
    }

    pub fn diagnostics_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running diagnostics…",
            Language::Indonesian => "Menjalankan diagnostik…",
            Language::German => "Diagnose läuft…",
        }
    }

    pub fn daemon_reloading(&self) -> &'static str {
        match self.lang {
            Language::English => "Reloading daemon configuration…",
            Language::Indonesian => "Memuat ulang konfigurasi daemon…",
            Language::German => "Daemon-Konfiguration wird neu geladen…",
        }
    }

    pub fn config_reload_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Loading configuration…",
            Language::Indonesian => "Memuat konfigurasi…",
            Language::German => "Konfiguration wird geladen…",
        }
    }

    pub fn status_reload_discarded_due_edits(&self) -> &'static str {
        match self.lang {
            Language::English => "Reload discarded because the configuration or draft changed while loading",
            Language::Indonesian => "Pemuatan ulang dibuang karena konfigurasi atau draf berubah saat dimuat",
            Language::German => "Neuladen verworfen, da sich Konfiguration oder Entwurf während des Ladens geändert hat",
        }
    }

    pub fn daemon_control_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Updating daemon state…",
            Language::Indonesian => "Memperbarui status daemon…",
            Language::German => "Daemon-Status wird aktualisiert…",
        }
    }

    pub fn background_queue_full(&self) -> &'static str {
        match self.lang {
            Language::English => "Background task queue is busy; try again shortly",
            Language::Indonesian => {
                "Antrean tugas latar belakang sedang sibuk; coba lagi sebentar lagi"
            }
            Language::German => {
                "Hintergrundwarteschlange ausgelastet; bitte gleich erneut versuchen"
            }
        }
    }

    pub fn background_runtime_stopped(&self) -> &'static str {
        match self.lang {
            Language::English => "Background runtime stopped unexpectedly",
            Language::Indonesian => "Runtime latar belakang berhenti secara tak terduga",
            Language::German => "Hintergrunddienst wurde unerwartet beendet",
        }
    }

    pub fn status_daemon_not_reloaded(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("daemon did not reload: {detail}"),
            Language::Indonesian => format!("daemon tidak memuat ulang: {detail}"),
            Language::German => format!("Daemon hat nicht neu geladen: {detail}"),
        }
    }

    pub fn status_buffer_limit_not_a_number(&self, detail: &str) -> String {
        match self.lang {
            Language::English => {
                format!("buffer limit must be a whole number ({detail})")
            }
            Language::Indonesian => {
                format!("batas buffer harus berupa bilangan bulat ({detail})")
            }
            Language::German => format!("Puffergrenze muss eine ganze Zahl sein ({detail})"),
        }
    }

    pub fn status_settings_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings invalid: {detail}"),
            Language::Indonesian => format!("Pengaturan tidak valid: {detail}"),
            Language::German => format!("Einstellungen ungültig: {detail}"),
        }
    }

    pub fn status_settings_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings rejected: {detail}"),
            Language::Indonesian => format!("Pengaturan ditolak: {detail}"),
            Language::German => format!("Einstellungen abgelehnt: {detail}"),
        }
    }

    pub fn status_settings_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Settings saved atomically",
            Language::Indonesian => "Pengaturan disimpan secara atomik",
            Language::German => "Einstellungen atomar gespeichert",
        }
    }

    pub fn status_appearance_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Appearance preference could not be saved: {detail}")
            }
            Language::Indonesian => {
                format!("Preferensi tampilan tidak dapat disimpan: {detail}")
            }
            Language::German => {
                format!("Darstellungseinstellung konnte nicht gespeichert werden: {detail}")
            }
        }
    }

    pub fn status_settings_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Settings save failed: {detail}"),
            Language::Indonesian => format!("Gagal menyimpan pengaturan: {detail}"),
            Language::German => format!("Speichern der Einstellungen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_font_size_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Font size saved",
            Language::Indonesian => "Ukuran huruf disimpan",
            Language::German => "Schriftgröße gespeichert",
        }
    }

    pub fn status_import_loaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Espanso library loaded for review",
            Language::Indonesian => "Pustaka Espanso dimuat untuk ditinjau",
            Language::German => "Espanso-Bibliothek zur Prüfung geladen",
        }
    }

    pub fn status_import_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import failed: {detail}"),
            Language::Indonesian => format!("Impor gagal: {detail}"),
            Language::German => format!("Import fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_import_needs_clean_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "Save or discard the current draft before importing",
            Language::Indonesian => "Simpan atau buang draf saat ini sebelum mengimpor",
            Language::German => "Aktuellen Entwurf vor dem Import speichern oder verwerfen",
        }
    }

    pub fn status_import_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import rejected: {detail}"),
            Language::Indonesian => format!("Impor ditolak: {detail}"),
            Language::German => format!("Import abgelehnt: {detail}"),
        }
    }

    pub fn status_import_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import save failed: {detail}"),
            Language::Indonesian => format!("Gagal menyimpan impor: {detail}"),
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
            Language::Indonesian => format!(
                "Impor Espanso diterapkan: {fully_migrated} termigrasi penuh, {with_warnings} dengan peringatan, {unsupported} tidak didukung"
            ),
            Language::German => format!(
                "Espanso-Import angewendet: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn status_config_reloaded(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration reloaded",
            Language::Indonesian => "Konfigurasi dimuat ulang",
            Language::German => "Konfiguration neu geladen",
        }
    }

    pub fn status_config_changed_externally(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration changed outside WayExpand; reload it before saving",
            Language::Indonesian => {
                "Konfigurasi diubah di luar WayExpand; muat ulang sebelum menyimpan"
            }
            Language::German => {
                "Konfiguration wurde außerhalb von WayExpand geändert; vor dem Speichern neu laden"
            }
        }
    }

    pub fn status_reload_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Reload failed: {detail}"),
            Language::Indonesian => format!("Gagal memuat ulang: {detail}"),
            Language::German => format!("Neu laden fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_invalid(&self, detail: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Command settings invalid: {detail}")
            }
            Language::Indonesian => {
                format!("Pengaturan perintah tidak valid: {detail}")
            }
            Language::German => format!("Befehlseinstellungen ungültig: {detail}"),
        }
    }

    pub fn status_save_rejected(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save rejected: {detail}"),
            Language::Indonesian => format!("Penyimpanan ditolak: {detail}"),
            Language::German => format!("Speichern abgelehnt: {detail}"),
        }
    }

    pub fn status_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Save failed: {detail}"),
            Language::Indonesian => format!("Gagal menyimpan: {detail}"),
            Language::German => format!("Speichern fehlgeschlagen: {detail}"),
        }
    }

    /// A change that was not written because the same items were changed
    /// outside WayExpand. The external version is loaded; the editor keeps
    /// the user's draft.
    pub fn status_not_saved_conflict(&self, items: &[String]) -> String {
        let items = items.join(", ");
        match self.lang {
            Language::English => format!(
                "Not saved: {items} also changed outside WayExpand. The external version is loaded and your edits remain in the editor; save again to overwrite, or revert to keep it."
            ),
            Language::Indonesian => format!(
                "Tidak disimpan: {items} juga diubah di luar WayExpand. Versi eksternal dimuat dan suntingan Anda tetap di editor; simpan lagi untuk menimpa, atau kembalikan untuk mempertahankannya."
            ),
            Language::German => format!(
                "Nicht gespeichert: {items} wurde auch außerhalb von WayExpand geändert. Die externe Version ist geladen, Ihre Änderungen bleiben im Editor; erneut speichern zum Überschreiben oder verwerfen, um sie zu behalten."
            ),
        }
    }

    pub fn status_not_saved_busy(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Not saved: another program kept the configuration locked. Your edits remain in the editor; try saving again."
            }
            Language::Indonesian => {
                "Tidak disimpan: program lain terus mengunci konfigurasi. Suntingan Anda tetap di editor; coba simpan lagi."
            }
            Language::German => {
                "Nicht gespeichert: Ein anderes Programm hielt die Konfiguration gesperrt. Ihre Änderungen bleiben im Editor; erneut speichern."
            }
        }
    }

    pub fn status_saving(&self) -> &'static str {
        match self.lang {
            Language::English => "Saving configuration…",
            Language::Indonesian => "Menyimpan konfigurasi…",
            Language::German => "Konfiguration wird gespeichert…",
        }
    }

    pub fn status_save_busy(&self) -> &'static str {
        match self.lang {
            Language::English => "A save is already in progress",
            Language::Indonesian => "Penyimpanan sedang berlangsung",
            Language::German => "Ein Speichervorgang läuft bereits",
        }
    }

    pub fn status_save_completed_with_newer_edits(&self) -> &'static str {
        match self.lang {
            Language::English => "Save completed; newer edits remain unsaved",
            Language::Indonesian => "Penyimpanan selesai; suntingan yang lebih baru belum disimpan",
            Language::German => "Gespeichert; neuere Änderungen sind noch nicht gespeichert",
        }
    }

    pub fn status_snippet_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet saved atomically",
            Language::Indonesian => "Cuplikan disimpan secara atomik",
            Language::German => "Snippet atomar gespeichert",
        }
    }

    pub fn status_nothing_to_undo(&self) -> &'static str {
        match self.lang {
            Language::English => "Nothing to undo",
            Language::Indonesian => "Tidak ada yang bisa diurungkan",
            Language::German => "Nichts zum Rückgängigmachen",
        }
    }

    pub fn status_undo_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Undo save failed: {detail}"),
            Language::Indonesian => format!("Gagal mengurungkan penyimpanan: {detail}"),
            Language::German => format!("Rückgängig-Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_undone(&self) -> &'static str {
        match self.lang {
            Language::English => "Undid the last saved change",
            Language::Indonesian => "Perubahan tersimpan terakhir diurungkan",
            Language::German => "Letzte gespeicherte Änderung rückgängig gemacht",
        }
    }

    pub fn status_created(&self) -> &'static str {
        match self.lang {
            Language::English => "Created a new snippet",
            Language::Indonesian => "Cuplikan baru dibuat",
            Language::German => "Neues Snippet erstellt",
        }
    }

    pub fn status_create_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Create failed: {detail}"),
            Language::Indonesian => format!("Gagal membuat: {detail}"),
            Language::German => format!("Erstellen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_duplicated(&self) -> &'static str {
        match self.lang {
            Language::English => "Duplicated snippet",
            Language::Indonesian => "Cuplikan diduplikasi",
            Language::German => "Snippet dupliziert",
        }
    }

    pub fn status_duplicate_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Duplicate failed: {detail}"),
            Language::Indonesian => format!("Gagal menduplikasi: {detail}"),
            Language::German => format!("Duplizieren fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_deleted(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Deleted {trigger}"),
            Language::Indonesian => format!("{trigger} dihapus"),
            Language::German => format!("{trigger} gelöscht"),
        }
    }

    pub fn status_delete_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Delete failed: {detail}"),
            Language::Indonesian => format!("Gagal menghapus: {detail}"),
            Language::German => format!("Löschen fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_snippet_enabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} enabled"),
            Language::Indonesian => format!("{trigger} diaktifkan"),
            Language::German => format!("{trigger} aktiviert"),
        }
    }

    pub fn status_snippet_disabled(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger} disabled"),
            Language::Indonesian => format!("{trigger} dinonaktifkan"),
            Language::German => format!("{trigger} deaktiviert"),
        }
    }

    pub fn status_toggle_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Toggle failed: {detail}"),
            Language::Indonesian => format!("Gagal mengubah status: {detail}"),
            Language::German => format!("Umschalten fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_paused(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion paused",
            Language::Indonesian => "Ekspansi dijeda",
            Language::German => "Erweiterung angehalten",
        }
    }

    pub fn status_resumed(&self) -> &'static str {
        match self.lang {
            Language::English => "Expansion resumed",
            Language::Indonesian => "Ekspansi dilanjutkan",
            Language::German => "Erweiterung fortgesetzt",
        }
    }

    pub fn status_control_unavailable(&self, detail: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Daemon control unavailable: {detail}")
            }
            Language::Indonesian => {
                format!("Kontrol daemon tidak tersedia: {detail}")
            }
            Language::German => format!("Daemon-Steuerung nicht verfügbar: {detail}"),
        }
    }

    pub fn status_app_filter_added(&self, value: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Added \"{value}\" to the app filter")
            }
            Language::Indonesian => {
                format!("\"{value}\" ditambahkan ke filter aplikasi")
            }
            Language::German => format!("\"{value}\" zum App-Filter hinzugefügt"),
        }
    }

    pub fn status_window_unidentified(&self) -> &'static str {
        match self.lang {
            Language::English => "Could not identify the focused window",
            Language::Indonesian => "Tidak dapat mengidentifikasi jendela yang fokus",
            Language::German => "Fokussiertes Fenster konnte nicht ermittelt werden",
        }
    }

    pub fn status_no_focused_window(&self) -> &'static str {
        match self.lang {
            Language::English => "No focused window to detect (focus is on the desktop)",
            Language::Indonesian => {
                "Tidak ada jendela fokus untuk dideteksi (fokus ada di desktop)"
            }
            Language::German => {
                "Kein fokussiertes Fenster erkennbar (Fokus liegt auf der Arbeitsfläche)"
            }
        }
    }

    pub fn status_detection_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection is unavailable here (KDE Plasma only for now)",
            Language::Indonesian => {
                "Deteksi jendela tidak tersedia di sini (saat ini hanya KDE Plasma)"
            }
            Language::German => {
                "Fenstererkennung ist hier nicht verfügbar (derzeit nur KDE Plasma)"
            }
        }
    }

    pub fn status_detection_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection failed unexpectedly",
            Language::Indonesian => "Deteksi jendela gagal secara tak terduga",
            Language::German => "Fenstererkennung ist unerwartet fehlgeschlagen",
        }
    }

    pub fn status_preview_copied(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview copied to clipboard",
            Language::Indonesian => "Pratinjau disalin ke papan klip",
            Language::German => "Vorschau in die Zwischenablage kopiert",
        }
    }

    pub fn status_enable_command_first(&self) -> &'static str {
        match self.lang {
            Language::English => "Enable the dynamic command first",
            Language::Indonesian => "Aktifkan perintah dinamis terlebih dahulu",
            Language::German => "Zuerst den dynamischen Befehl aktivieren",
        }
    }

    pub fn status_command_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command failed: {detail}"),
            Language::Indonesian => format!("Perintah gagal: {detail}"),
            Language::German => format!("Befehl fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_command_preview_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Command preview failed unexpectedly",
            Language::Indonesian => "Pratinjau perintah gagal secara tak terduga",
            Language::German => "Befehlsvorschau ist unerwartet fehlgeschlagen",
        }
    }

    pub fn unsaved_change_kept(&self) -> &'static str {
        match self.lang {
            Language::English => "A change is not saved yet",
            Language::Indonesian => "Ada perubahan yang belum disimpan",
            Language::German => "Eine Änderung ist noch nicht gespeichert",
        }
    }

    pub fn retry_save(&self) -> &'static str {
        match self.lang {
            Language::English => "Retry save",
            Language::Indonesian => "Coba simpan lagi",
            Language::German => "Erneut speichern",
        }
    }

    pub fn discard_unsaved_change(&self) -> &'static str {
        match self.lang {
            Language::English => "Discard change",
            Language::Indonesian => "Buang perubahan",
            Language::German => "Änderung verwerfen",
        }
    }

    pub fn status_unsaved_change_discarded(&self) -> &'static str {
        match self.lang {
            Language::English => "Unsaved change discarded",
            Language::Indonesian => "Perubahan yang belum disimpan dibuang",
            Language::German => "Nicht gespeicherte Änderung verworfen",
        }
    }

    pub fn status_unsaved_change_blocks_close(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "A change is not saved yet. Retry or discard it, or close again to quit without it."
            }
            Language::Indonesian => {
                "Ada perubahan yang belum disimpan. Coba simpan lagi atau buang, atau tutup sekali lagi untuk keluar tanpa perubahan itu."
            }
            Language::German => {
                "Eine Änderung ist noch nicht gespeichert. Erneut speichern oder verwerfen, oder erneut schließen, um ohne sie zu beenden."
            }
        }
    }
}
