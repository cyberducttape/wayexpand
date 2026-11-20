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
