use super::{Language, Strings};

impl Strings {
    fn pick(&self, en: &'static str, de: &'static str, id: &'static str) -> &'static str {
        match self.lang {
            Language::English => en,
            Language::German => de,
            Language::Indonesian => id,
        }
    }

    pub fn recovery_title(&self) -> &'static str {
        self.pick(
            "WayExpand can't open your snippet library",
            "WayExpand kann Ihre Snippet-Bibliothek nicht öffnen",
            "WayExpand tidak dapat membuka pustaka cuplikan Anda",
        )
    }

    pub fn recovery_intro(&self) -> &'static str {
        self.pick(
            "The configuration file failed validation. Nothing has been changed. Choose how to repair it:",
            "Die Konfigurationsdatei hat die Prüfung nicht bestanden. Es wurde nichts verändert. Wählen Sie, wie Sie sie reparieren möchten:",
            "Berkas konfigurasi gagal divalidasi. Belum ada yang diubah. Pilih cara memperbaikinya:",
        )
    }

    pub fn recovery_fix_heading(&self) -> &'static str {
        self.pick("Fix the file", "Datei reparieren", "Perbaiki berkas")
    }

    pub fn recovery_open_file(&self) -> &'static str {
        self.pick(
            "Open in a text editor",
            "In einem Texteditor öffnen",
            "Buka di editor teks",
        )
    }

    pub fn recovery_open_folder(&self) -> &'static str {
        self.pick("Open its folder", "Ordner öffnen", "Buka foldernya")
    }

    pub fn recovery_try_again(&self) -> &'static str {
        self.pick("Try again", "Erneut versuchen", "Coba lagi")
    }

    pub fn recovery_backups_heading(&self) -> &'static str {
        self.pick(
            "Restore a backup",
            "Sicherung wiederherstellen",
            "Pulihkan cadangan",
        )
    }

    pub fn recovery_no_backups(&self) -> &'static str {
        self.pick(
            "No backups were found next to the configuration file.",
            "Neben der Konfigurationsdatei wurden keine Sicherungen gefunden.",
            "Tidak ada cadangan di samping berkas konfigurasi.",
        )
    }

    pub fn recovery_restore(&self) -> &'static str {
        self.pick(
            "Restore this backup",
            "Diese Sicherung wiederherstellen",
            "Pulihkan cadangan ini",
        )
    }

    pub fn recovery_backup_invalid(&self) -> &'static str {
        self.pick("not valid either", "ebenfalls ungültig", "juga tidak valid")
    }

    pub fn recovery_fresh_heading(&self) -> &'static str {
        self.pick("Start over", "Neu beginnen", "Mulai dari awal")
    }

    pub fn recovery_fresh_detail(&self) -> &'static str {
        self.pick(
            "The broken file is kept beside the new one as a .broken copy, so nothing is lost.",
            "Die defekte Datei bleibt als .broken-Kopie neben der neuen erhalten, es geht also nichts verloren.",
            "Berkas yang rusak disimpan di samping yang baru sebagai salinan .broken, jadi tidak ada yang hilang.",
        )
    }

    pub fn recovery_fresh_button(&self) -> &'static str {
        self.pick(
            "Start a new, empty library",
            "Neue, leere Bibliothek anlegen",
            "Buat pustaka baru yang kosong",
        )
    }

    pub fn recovery_open_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not open it: {error}"),
            Language::German => format!("Konnte nicht geöffnet werden: {error}"),
            Language::Indonesian => format!("Tidak dapat dibuka: {error}"),
        }
    }

    pub fn recovery_relaunch_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not restart WayExpand: {error}"),
            Language::German => format!("WayExpand konnte nicht neu gestartet werden: {error}"),
            Language::Indonesian => format!("WayExpand tidak dapat dimulai ulang: {error}"),
        }
    }

    pub fn recovery_action_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("That did not work, and nothing was replaced: {error}"),
            Language::German => {
                format!("Das hat nicht funktioniert; nichts wurde ersetzt: {error}")
            }
            Language::Indonesian => {
                format!("Itu tidak berhasil, dan tidak ada yang diganti: {error}")
            }
        }
    }
}
