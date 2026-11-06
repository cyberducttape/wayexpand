use super::{Language, Strings};
use crate::runtime::{OutputRefusal, ReplacementAssurance};
use wayexpand_core::CompatibilityMode;

impl Strings {
    pub fn insertion_plan_notice(&self, strategy: wayexpand_core::InsertionStrategy) -> String {
        use wayexpand_core::InsertionStrategy as Strategy;
        match (self.lang, strategy) {
            (Language::English, Strategy::VerifiedReplacement) => {
                "Plan: verified atomic replacement.".into()
            }
            (Language::English, Strategy::NativeTextCommit) => {
                "Plan: native text commit. Compatibility mode is enabled; replacement remains non-atomic.".into()
            }
            (Language::English, Strategy::ClipboardAssistedPaste) => {
                "Plan: copy and paste manually. The active route cannot provide a verified, interactive replacement. For long configuration text, use `wayexpand new` to generate a file.".into()
            }
            (Language::English, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Plan: paced synthetic keystrokes, estimated {predicted_ms} ms; compatibility mode is enabled.")
            }
            (Language::German, Strategy::VerifiedReplacement) => {
                "Plan: verifizierte atomare Ersetzung.".into()
            }
            (Language::German, Strategy::NativeTextCommit) => {
                "Plan: native Texteingabe. Der Kompatibilitätsmodus ist aktiv; die Ersetzung bleibt nicht atomar.".into()
            }
            (Language::German, Strategy::ClipboardAssistedPaste) => {
                "Plan: manuell kopieren und einfügen. Der aktive Weg bietet keine verifizierte interaktive Ersetzung.".into()
            }
            (Language::German, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Plan: simulierte Tastenanschläge, geschätzt {predicted_ms} ms; der Kompatibilitätsmodus ist aktiv.")
            }
            (Language::Indonesian, Strategy::VerifiedReplacement) => {
                "Rencana: penggantian atomik terverifikasi.".into()
            }
            (Language::Indonesian, Strategy::NativeTextCommit) => {
                "Rencana: komit teks native. Mode kompatibilitas aktif; penggantian tetap non-atomik.".into()
            }
            (Language::Indonesian, Strategy::ClipboardAssistedPaste) => {
                "Rencana: salin dan tempel secara manual. Jalur aktif tidak menyediakan penggantian interaktif terverifikasi.".into()
            }
            (Language::Indonesian, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Rencana: ketukan papan ketik sintetis, perkiraan {predicted_ms} ms; mode kompatibilitas aktif.")
            }
        }
    }

    pub fn replacement_pill(&self, assurance: ReplacementAssurance) -> &'static str {
        match (self.lang, assurance) {
            (Language::English, ReplacementAssurance::Verified) => {
                "Replacement: Target text verified"
            }
            (Language::English, ReplacementAssurance::FocusTrackedBestEffort) => {
                "Replacement: Focus-tracked best effort"
            }
            (Language::English, ReplacementAssurance::Unverified) => "Replacement: Unverified",
            (Language::English, ReplacementAssurance::Blocked) => "Replacement: Blocked",
            (Language::English, ReplacementAssurance::Unavailable) => "Replacement: Unavailable",
            (Language::German, ReplacementAssurance::Verified) => "Ersetzung: Zieltext verifiziert",
            (Language::German, ReplacementAssurance::FocusTrackedBestEffort) => {
                "Ersetzung: Fokusgeprüft, bestmöglich"
            }
            (Language::German, ReplacementAssurance::Unverified) => "Ersetzung: Unverifiziert",
            (Language::German, ReplacementAssurance::Blocked) => "Ersetzung: Blockiert",
            (Language::German, ReplacementAssurance::Unavailable) => "Ersetzung: Nicht verfügbar",
            (Language::Indonesian, ReplacementAssurance::Verified) => {
                "Penggantian: Teks tujuan terverifikasi"
            }
            (Language::Indonesian, ReplacementAssurance::FocusTrackedBestEffort) => {
                "Penggantian: Upaya terbaik dengan pelacakan fokus"
            }
            (Language::Indonesian, ReplacementAssurance::Unverified) => {
                "Penggantian: Tidak terverifikasi"
            }
            (Language::Indonesian, ReplacementAssurance::Blocked) => "Penggantian: Diblokir",
            (Language::Indonesian, ReplacementAssurance::Unavailable) => {
                "Penggantian: Tidak tersedia"
            }
        }
    }

    /// Explain exactly what the active route can establish before erasing.
    pub fn replacement_explanation(&self, assurance: ReplacementAssurance) -> &'static str {
        match (self.lang, assurance) {
            (Language::English, ReplacementAssurance::Verified) => "The route checks the target field's surrounding text before deleting the trigger.",
            (Language::English, ReplacementAssurance::FocusTrackedBestEffort) => "The window tracker confirms focus immediately before typing, but the route cannot verify that the text at the cursor is still the trigger. New physical typing can reach the app during replacement and may delete surrounding text or leave part of the trigger behind. A focus change refuses or stops output.",
            (Language::English, ReplacementAssurance::Unverified) => "This explicitly allowed keystroke route cannot verify the target field, focus, or text it erases. New physical typing can reach the app during replacement and may delete surrounding text or leave part of the trigger behind. Strict mode refuses this operation.",
            (Language::English, ReplacementAssurance::Blocked) => "Strict mode refuses keystroke replacement because the route cannot verify what it erases. Choose a verified route or explicitly allow a compatibility mode in Settings → Engine.",
            (Language::English, ReplacementAssurance::Unavailable) => "No output route is connected.",
            (Language::German, ReplacementAssurance::Verified) => "Der Weg prüft den umgebenden Text des Zielfelds, bevor der Auslöser gelöscht wird.",
            (Language::German, ReplacementAssurance::FocusTrackedBestEffort) => "Der Fenster-Tracker bestätigt den Fokus direkt vor dem Tippen, aber der Weg kann nicht prüfen, ob am Cursor noch der Auslöser steht. Neue physische Eingaben können die Anwendung während der Ersetzung erreichen und umliegenden Text löschen oder Teile des Auslösers zurücklassen. Ein Fokuswechsel verweigert oder stoppt die Ausgabe.",
            (Language::German, ReplacementAssurance::Unverified) => "Dieser ausdrücklich erlaubte Tastenanschlag-Weg kann Zielfeld, Fokus oder gelöschten Text nicht verifizieren. Neue physische Eingaben können die Anwendung während der Ersetzung erreichen und umliegenden Text löschen oder Teile des Auslösers zurücklassen. Der strikte Modus verweigert diesen Vorgang.",
            (Language::German, ReplacementAssurance::Blocked) => "Der strikte Modus verweigert die Ersetzung per Tastenanschlag, weil der Weg den gelöschten Text nicht prüfen kann. Wählen Sie einen verifizierten Weg oder erlauben Sie ausdrücklich einen Kompatibilitätsmodus unter Einstellungen → Engine.",
            (Language::German, ReplacementAssurance::Unavailable) => "Kein Ausgabeweg ist verbunden.",
            (Language::Indonesian, ReplacementAssurance::Verified) => "Jalur ini memeriksa teks di sekitar pemicu pada kolom tujuan sebelum menghapusnya.",
            (Language::Indonesian, ReplacementAssurance::FocusTrackedBestEffort) => "Pelacak jendela memastikan fokus tepat sebelum mengetik, tetapi jalur ini tidak dapat memastikan teks di kursor masih berupa pemicu. Ketikan fisik baru dapat masuk ke aplikasi selama penggantian dan mungkin menghapus teks di sekitarnya atau menyisakan sebagian pemicu. Perpindahan fokus akan menolak atau menghentikan keluaran.",
            (Language::Indonesian, ReplacementAssurance::Unverified) => "Jalur tombol yang diizinkan secara eksplisit ini tidak dapat memastikan kolom tujuan, fokus, atau teks yang dihapus. Ketikan fisik baru dapat masuk ke aplikasi selama penggantian dan mungkin menghapus teks di sekitarnya atau menyisakan sebagian pemicu. Mode ketat menolak operasi ini.",
            (Language::Indonesian, ReplacementAssurance::Blocked) => "Mode ketat menolak penggantian melalui tombol karena jalur ini tidak dapat memastikan teks yang dihapus. Pilih jalur terverifikasi atau izinkan mode kompatibilitas secara eksplisit di Pengaturan → Mesin.",
            (Language::Indonesian, ReplacementAssurance::Unavailable) => "Tidak ada jalur keluaran yang terhubung.",
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
                "The result of {count} output transaction(s) is unknown. They were not retried; check the affected text and recent outcomes."
            ),
            Language::German => format!(
                "Das Ergebnis von {count} Ausgabevorgang(en) ist unbekannt. Sie wurden nicht wiederholt; prüfen Sie den betroffenen Text und die letzten Ergebnisse."
            ),
            Language::Indonesian => format!(
                "Hasil {count} transaksi keluaran tidak diketahui. Transaksi tidak diulang; periksa teks dan hasil terbaru."
            ),
        }
    }

    pub fn output_partial_notice(&self, count: u64) -> String {
        match self.lang {
            Language::English => format!("Partial insertion detected in {count} transaction(s). Check the target text; WayExpand did not retry."),
            Language::German => format!("Teilweise Einfügung bei {count} Vorgang/Vorgängen erkannt. Prüfen Sie den Zieltext; WayExpand hat nichts wiederholt."),
            Language::Indonesian => format!("Penyisipan sebagian terdeteksi pada {count} transaksi. Periksa teks tujuan; WayExpand tidak mengulanginya."),
        }
    }

    pub fn output_partial_badge(&self, count: u64) -> String {
        match self.lang {
            Language::English => format!("Partial insertion · {count}"),
            Language::German => format!("Teilweise Einfügung · {count}"),
            Language::Indonesian => format!("Penyisipan sebagian · {count}"),
        }
    }

    pub fn output_uncertain_badge(&self, count: u64) -> String {
        match self.lang {
            Language::English => format!("Check output · {count}"),
            Language::German => format!("Ausgabe prüfen · {count}"),
            Language::Indonesian => format!("Periksa keluaran · {count}"),
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
                "Characters (1–5000). When text must be typed key by key (the libei keysym fallback, about 83 characters a second), longer replacements are refused. The default limit is 8 characters (about 100 ms). Longer output is refused unless you raise the limit explicitly; copy and paste or generate a file for bulk text."
            }
            Language::German => {
                "Zeichen (1–5000). Muss Text Taste für Taste getippt werden (libei-Keysym-Fallback, etwa 83 Zeichen pro Sekunde), werden längere Ersetzungen abgelehnt. Der Standardwert beträgt 8 Zeichen (etwa 100 ms). Längere Ausgabe wird abgelehnt, sofern Sie das Limit nicht ausdrücklich erhöhen; kopieren und einfügen oder erzeugen Sie eine Datei für größere Texte."
            }
            Language::Indonesian => {
                "Karakter (1–5000). Jika teks harus diketik tombol demi tombol (fallback keysym libei, sekitar 83 karakter per detik), penggantian yang lebih panjang ditolak. Batas bawaan adalah 8 karakter (sekitar 100 ms). Keluaran lebih panjang ditolak kecuali batas dinaikkan secara eksplisit; salin dan tempel atau buat berkas untuk teks besar."
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
                "Verified routes (input method, IBus) check the field's own text before replacing a trigger. Keystroke routes (evdev with libei or wlroots) erase with Backspace without seeing the text, so they replace triggers only if you allow it here. Tracked focus also requires the window tracker (KWin, or the GNOME Shell extension) to confirm, right before typing, that the trigger's window still has focus. Untracked focus allows erasing when no tracker can confirm focus. A detected focus change is always refused, and an output that may have been partly typed is never retried."
            }
            Language::German => {
                "Verifizierte Wege (Eingabemethode, IBus) prüfen den Text des Feldes, bevor ein Auslöser ersetzt wird. Tastenanschlag-Wege (evdev mit libei oder wlroots) löschen per Rücktaste, ohne den Text zu sehen, und ersetzen Auslöser daher nur, wenn Sie es hier erlauben. Verfolgter Fokus verlangt zusätzlich, dass die Fensterverfolgung (KWin oder die GNOME-Shell-Erweiterung) direkt vor dem Tippen bestätigt, dass das Fenster des Auslösers noch fokussiert ist. Unverfolgter Fokus erlaubt das Löschen auch ohne bestätigten Fokus. Ein erkannter Fokuswechsel wird immer abgelehnt, und eine möglicherweise nur teilweise getippte Ausgabe wird nie wiederholt."
            }
            Language::Indonesian => {
                "Jalur terverifikasi (metode input, IBus) memeriksa teks kolom itu sendiri sebelum mengganti pemicu. Jalur penekanan tombol (evdev dengan libei atau wlroots) menghapus dengan Backspace tanpa melihat teks, sehingga hanya mengganti pemicu jika Anda mengizinkannya di sini. Fokus terlacak juga mengharuskan pelacak jendela (KWin, atau ekstensi GNOME Shell) memastikan, tepat sebelum mengetik, bahwa jendela pemicu masih fokus. Fokus tak terlacak mengizinkan penghapusan saat tidak ada pelacak yang dapat memastikan fokus. Perpindahan fokus yang terdeteksi selalu ditolak, dan keluaran yang mungkin hanya sebagian terketik tidak pernah diulang."
            }
        }
    }

    pub fn compatibility_mode_risk(&self) -> &'static str {
        match self.lang {
            Language::English => "Risk of non-atomic replacement: physical typing can reach the application while WayExpand is erasing and inserting, so the two streams may interleave and corrupt nearby text. Evdev also cannot detect password fields. Keep this Off for strict safety; prefer a verified input-method route when text integrity matters.",
            Language::German => "Risiko einer nicht atomaren Ersetzung: Physische Tasten können die Anwendung erreichen, während WayExpand löscht und einfügt. Beide Eingabeströme können sich überlagern und benachbarten Text beschädigen. Evdev erkennt außerdem keine Passwortfelder. Für strikte Sicherheit deaktiviert lassen; wenn Textintegrität wichtig ist, eine verifizierte Eingabemethoden-Route verwenden.",
            Language::Indonesian => "Risiko penggantian non-atomik: ketikan fisik dapat mencapai aplikasi saat WayExpand menghapus dan menyisipkan teks. Kedua aliran input dapat bercampur dan merusak teks di sekitarnya. Evdev juga tidak dapat mendeteksi kolom kata sandi. Biarkan nonaktif untuk keselamatan ketat; gunakan rute metode input terverifikasi bila integritas teks penting.",
        }
    }
}
