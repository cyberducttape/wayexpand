use super::{Language, Strings};
use crate::runtime::{InsertionAssurance, OutputRefusal};
use wayexpand_core::CompatibilityMode;

impl Strings {
    pub fn insertion_pill(&self, assurance: InsertionAssurance) -> &'static str {
        match (self.lang, assurance) {
            (Language::English, InsertionAssurance::Verified) => "Insertion: Verified",
            (Language::English, InsertionAssurance::Compatibility) => "Insertion: Compatibility",
            (Language::English, InsertionAssurance::Blocked) => "Insertion: Blocked",
            (Language::English, InsertionAssurance::Unavailable) => "Insertion: Unavailable",
            (Language::German, InsertionAssurance::Verified) => "Einfügen: Verifiziert",
            (Language::German, InsertionAssurance::Compatibility) => "Einfügen: Kompatibilität",
            (Language::German, InsertionAssurance::Blocked) => "Einfügen: Blockiert",
            (Language::German, InsertionAssurance::Unavailable) => "Einfügen: Nicht verfügbar",
            (Language::Indonesian, InsertionAssurance::Verified) => "Penyisipan: Terverifikasi",
            (Language::Indonesian, InsertionAssurance::Compatibility) => {
                "Penyisipan: Kompatibilitas"
            }
            (Language::Indonesian, InsertionAssurance::Blocked) => "Penyisipan: Diblokir",
            (Language::Indonesian, InsertionAssurance::Unavailable) => "Penyisipan: Tidak tersedia",
        }
    }

    /// What the active route promises about the text it replaces.
    /// `focus_tracked` is whether a window tracker confirms focus right now.
    pub fn insertion_explanation(
        &self,
        assurance: InsertionAssurance,
        focus_tracked: bool,
    ) -> &'static str {
        match (self.lang, assurance, focus_tracked) {
            (Language::English, InsertionAssurance::Verified, _) => {
                "Verified insertion: the input method checks the field's own text before replacing a trigger, and the edit is bound to that field."
            }
            (Language::English, InsertionAssurance::Compatibility, true) => {
                "Compatibility mode: triggers are erased with keystrokes without seeing the field's text. The window tracker confirms focus right before typing; a focus change cancels the expansion."
            }
            (Language::English, InsertionAssurance::Compatibility, false) => {
                "Compatibility mode: triggers are erased with keystrokes without seeing the field's text, and focus is not confirmed, so the text goes to whatever has focus."
            }
            (Language::English, InsertionAssurance::Blocked, _) => {
                "Typed expansions are refused: this keystroke route cannot verify what it erases. Use a verified route, or allow compatibility mode in Settings → Engine."
            }
            (Language::English, InsertionAssurance::Unavailable, _) => {
                "No output route is connected."
            }
            (Language::German, InsertionAssurance::Verified, _) => {
                "Verifiziertes Einfügen: Die Eingabemethode prüft den Text des Feldes, bevor ein Auslöser ersetzt wird, und die Änderung ist an dieses Feld gebunden."
            }
            (Language::German, InsertionAssurance::Compatibility, true) => {
                "Kompatibilitätsmodus: Auslöser werden per Tastenanschlag gelöscht, ohne den Text des Feldes zu sehen. Der Fenster-Tracker bestätigt den Fokus direkt vor dem Tippen; ein Fokuswechsel bricht die Erweiterung ab."
            }
            (Language::German, InsertionAssurance::Compatibility, false) => {
                "Kompatibilitätsmodus: Auslöser werden per Tastenanschlag gelöscht, ohne den Text des Feldes zu sehen, und der Fokus wird nicht bestätigt. Der Text landet im gerade fokussierten Element."
            }
            (Language::German, InsertionAssurance::Blocked, _) => {
                "Getippte Erweiterungen werden abgelehnt: Dieser Tastenanschlag-Weg kann nicht prüfen, was er löscht. Verwenden Sie einen verifizierten Weg oder erlauben Sie den Kompatibilitätsmodus unter Einstellungen → Engine."
            }
            (Language::German, InsertionAssurance::Unavailable, _) => {
                "Kein Ausgabeweg ist verbunden."
            }
            (Language::Indonesian, InsertionAssurance::Verified, _) => {
                "Penyisipan terverifikasi: metode input memeriksa teks kolom itu sendiri sebelum mengganti pemicu, dan suntingannya terikat pada kolom tersebut."
            }
            (Language::Indonesian, InsertionAssurance::Compatibility, true) => {
                "Mode kompatibilitas: pemicu dihapus dengan penekanan tombol tanpa melihat teks kolom. Pelacak jendela memastikan fokus tepat sebelum mengetik; perpindahan fokus membatalkan ekspansi."
            }
            (Language::Indonesian, InsertionAssurance::Compatibility, false) => {
                "Mode kompatibilitas: pemicu dihapus dengan penekanan tombol tanpa melihat teks kolom, dan fokus tidak dipastikan, sehingga teks masuk ke elemen mana pun yang sedang fokus."
            }
            (Language::Indonesian, InsertionAssurance::Blocked, _) => {
                "Ekspansi yang diketik ditolak: jalur penekanan tombol ini tidak dapat memverifikasi apa yang dihapusnya. Gunakan jalur terverifikasi, atau izinkan mode kompatibilitas di Pengaturan → Mesin."
            }
            (Language::Indonesian, InsertionAssurance::Unavailable, _) => {
                "Tidak ada jalur keluaran yang terhubung."
            }
        }
    }

    pub fn output_refused_notice(&self, count: u64, reason: Option<OutputRefusal>) -> String {
        let reason = match (self.lang, reason) {
            (Language::English, Some(OutputRefusal::CompatibilityModeOff)) => {
                "compatibility mode is off"
            }
            (Language::English, Some(OutputRefusal::FocusChanged)) => {
                "focus changed after the trigger"
            }
            (Language::English, Some(OutputRefusal::FocusUnconfirmed)) => {
                "focus could not be confirmed"
            }
            (Language::English, None) => "unknown",
            (Language::German, Some(OutputRefusal::CompatibilityModeOff)) => {
                "Kompatibilitätsmodus ist aus"
            }
            (Language::German, Some(OutputRefusal::FocusChanged)) => {
                "Fokus wechselte nach dem Auslöser"
            }
            (Language::German, Some(OutputRefusal::FocusUnconfirmed)) => {
                "Fokus konnte nicht bestätigt werden"
            }
            (Language::German, None) => "unbekannt",
            (Language::Indonesian, Some(OutputRefusal::CompatibilityModeOff)) => {
                "mode kompatibilitas nonaktif"
            }
            (Language::Indonesian, Some(OutputRefusal::FocusChanged)) => {
                "fokus berpindah setelah pemicu"
            }
            (Language::Indonesian, Some(OutputRefusal::FocusUnconfirmed)) => {
                "fokus tidak dapat dipastikan"
            }
            (Language::Indonesian, None) => "tidak diketahui",
        };
        match self.lang {
            Language::English => format!(
                "{count} expansion(s) were refused before typing (last reason: {reason}); nothing was erased or inserted."
            ),
            Language::German => format!(
                "{count} Erweiterung(en) wurden vor dem Tippen abgelehnt (letzter Grund: {reason}); nichts wurde gelöscht oder eingefügt."
            ),
            Language::Indonesian => format!(
                "{count} ekspansi ditolak sebelum mengetik (alasan terakhir: {reason}); tidak ada yang dihapus atau disisipkan."
            ),
        }
    }

    pub fn output_uncertain_notice(&self, count: u64) -> String {
        match self.lang {
            Language::English => format!(
                "{count} expansion(s) may have been partly typed when output failed. They were not retried; check the affected text."
            ),
            Language::German => format!(
                "{count} Erweiterung(en) wurden beim Ausgabefehler möglicherweise nur teilweise getippt. Sie wurden nicht wiederholt; prüfen Sie den betroffenen Text."
            ),
            Language::Indonesian => format!(
                "{count} ekspansi mungkin hanya sebagian terketik saat keluaran gagal. Ekspansi tersebut tidak diulang; periksa teks yang terdampak."
            ),
        }
    }

    pub fn keystroke_limit(&self) -> &'static str {
        match self.lang {
            Language::English => "Longest keystroke-typed replacement",
            Language::German => "Längste per Tastenanschlag getippte Ersetzung",
            Language::Indonesian => "Penggantian terpanjang yang diketik per tombol",
        }
    }

    pub fn keystroke_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Characters (1–5000). When text must be typed key by key (the libei keysym fallback, about 83 characters a second), longer replacements are refused. 250 is about 3 seconds; anything you type meanwhile is mixed into the replacement."
            }
            Language::German => {
                "Zeichen (1–5000). Muss Text Taste für Taste getippt werden (libei-Keysym-Fallback, etwa 83 Zeichen pro Sekunde), werden längere Ersetzungen abgelehnt. 250 sind etwa 3 Sekunden; was Sie währenddessen tippen, landet in der Ersetzung."
            }
            Language::Indonesian => {
                "Karakter (1–5000). Jika teks harus diketik tombol demi tombol (fallback keysym libei, sekitar 83 karakter per detik), penggantian yang lebih panjang ditolak. 250 sekitar 3 detik; apa pun yang Anda ketik selama itu tercampur ke penggantian."
            }
        }
    }

    pub fn status_keystroke_limit_not_a_number(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("keystroke limit must be a whole number ({detail})"),
            Language::German => {
                format!("Tastenanschlag-Limit muss eine ganze Zahl sein ({detail})")
            }
            Language::Indonesian => {
                format!("batas penekanan tombol harus berupa bilangan bulat ({detail})")
            }
        }
    }

    pub fn compatibility_mode_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Keystroke routes (compatibility mode)",
            Language::German => "Tastenanschlag-Wege (Kompatibilitätsmodus)",
            Language::Indonesian => "Jalur penekanan tombol (mode kompatibilitas)",
        }
    }

    pub fn compatibility_mode_option(&self, mode: CompatibilityMode) -> &'static str {
        match (self.lang, mode) {
            (Language::English, CompatibilityMode::Off) => "Off: verified routes only",
            (Language::English, CompatibilityMode::TrackedFocus) => "Tracked focus",
            (Language::English, CompatibilityMode::UntrackedFocus) => "Untracked focus",
            (Language::German, CompatibilityMode::Off) => "Aus: nur verifizierte Wege",
            (Language::German, CompatibilityMode::TrackedFocus) => "Verfolgter Fokus",
            (Language::German, CompatibilityMode::UntrackedFocus) => "Unverfolgter Fokus",
            (Language::Indonesian, CompatibilityMode::Off) => "Nonaktif: hanya jalur terverifikasi",
            (Language::Indonesian, CompatibilityMode::TrackedFocus) => "Fokus terlacak",
            (Language::Indonesian, CompatibilityMode::UntrackedFocus) => "Fokus tak terlacak",
        }
    }

    pub fn compatibility_mode_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Verified routes (input method, IBus) check the field's own text before replacing a trigger. Keystroke routes (evdev with libei or wlroots) erase with Backspace without seeing the text, so they replace triggers only if you allow it here. Tracked focus also requires the KDE window tracker to confirm, right before typing, that the trigger's window still has focus. Untracked focus allows erasing when no tracker can confirm focus. A detected focus change is always refused, and an output that may have been partly typed is never retried."
            }
            Language::German => {
                "Verifizierte Wege (Eingabemethode, IBus) prüfen den Text des Feldes, bevor ein Auslöser ersetzt wird. Tastenanschlag-Wege (evdev mit libei oder wlroots) löschen per Rücktaste, ohne den Text zu sehen, und ersetzen Auslöser daher nur, wenn Sie es hier erlauben. Verfolgter Fokus verlangt zusätzlich, dass der KDE-Fenster-Tracker direkt vor dem Tippen bestätigt, dass das Fenster des Auslösers noch fokussiert ist. Unverfolgter Fokus erlaubt das Löschen auch ohne bestätigten Fokus. Ein erkannter Fokuswechsel wird immer abgelehnt, und eine möglicherweise nur teilweise getippte Ausgabe wird nie wiederholt."
            }
            Language::Indonesian => {
                "Jalur terverifikasi (metode input, IBus) memeriksa teks kolom itu sendiri sebelum mengganti pemicu. Jalur penekanan tombol (evdev dengan libei atau wlroots) menghapus dengan Backspace tanpa melihat teks, sehingga hanya mengganti pemicu jika Anda mengizinkannya di sini. Fokus terlacak juga mengharuskan pelacak jendela KDE memastikan, tepat sebelum mengetik, bahwa jendela pemicu masih fokus. Fokus tak terlacak mengizinkan penghapusan saat tidak ada pelacak yang dapat memastikan fokus. Perpindahan fokus yang terdeteksi selalu ditolak, dan keluaran yang mungkin hanya sebagian terketik tidak pernah diulang."
            }
        }
    }
}
