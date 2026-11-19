use super::{Language, Strings};

impl Strings {
    // Status line

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

    pub fn status_import_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Import failed: {detail}"),
            Language::Indonesian => format!("Impor gagal: {detail}"),
            Language::German => format!("Import fehlgeschlagen: {detail}"),
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

    pub fn status_save_before_matcher_preview(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Save or discard the current snippet edits before opening the onboarding preview."
            }
            Language::Indonesian => {
                "Simpan atau buang perubahan cuplikan saat ini sebelum membuka pratinjau awal."
            }
            Language::German => {
                "Speichern oder verwerfen Sie die aktuellen Snippet-Änderungen, bevor Sie die Vorschau öffnen."
            }
        }
    }

    pub fn status_undo_save_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Undo save failed: {detail}"),
            Language::Indonesian => format!("Gagal mengurungkan penyimpanan: {detail}"),
            Language::German => format!("Rückgängig-Speichern fehlgeschlagen: {detail}"),
        }
    }

    pub fn status_create_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Create failed: {detail}"),
            Language::Indonesian => format!("Gagal membuat: {detail}"),
            Language::German => format!("Erstellen fehlgeschlagen: {detail}"),
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

    pub fn status_detection_timed_out(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "No other app was clicked within 15 seconds; click Use current app, then click the app's window"
            }
            Language::Indonesian => {
                "Tidak ada aplikasi lain yang diklik dalam 15 detik; klik Gunakan aplikasi saat ini, lalu klik jendela aplikasinya"
            }
            Language::German => {
                "Innerhalb von 15 Sekunden wurde keine andere App angeklickt; Aktuelle App verwenden und dann das Fenster der App anklicken"
            }
        }
    }

    pub fn status_detection_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Window detection is unavailable here (KDE Plasma, or GNOME with the WayExpand extension: run `wayexpand setup`)",
            Language::Indonesian => {
                "Deteksi jendela tidak tersedia di sini (KDE Plasma, atau GNOME dengan ekstensi WayExpand: jalankan `wayexpand setup`)"
            }
            Language::German => {
                "Fenstererkennung ist hier nicht verfügbar (KDE Plasma oder GNOME mit der WayExpand-Erweiterung: `wayexpand setup` ausführen)"
            }
        }
    }

    pub fn status_command_failed(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Command failed: {detail}"),
            Language::Indonesian => format!("Perintah gagal: {detail}"),
            Language::German => format!("Befehl fehlgeschlagen: {detail}"),
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
