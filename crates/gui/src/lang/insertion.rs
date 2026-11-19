use super::{Language, Strings};
use crate::runtime::{OutputRefusal, ReplacementAssurance};
use wayexpand_core::{CompatibilityMode, TerminalPaste};

impl Strings {
    pub fn insertion_plan_notice(&self, strategy: wayexpand_core::InsertionStrategy) -> String {
        use wayexpand_core::InsertionStrategy as Strategy;
        match (self.lang, strategy) {
            (Language::English, Strategy::VerifiedReplacement) => {
                "Plan: use the atomic route contract. Application-level insertion still needs a real-app test.".into()
            }
            (Language::English, Strategy::NativeTextCommit) => {
                "Plan: native text commit. Compatibility mode is enabled; replacement is non-atomic and the application result remains unverified.".into()
            }
            (Language::English, Strategy::ClipboardAssistedPaste) => {
                "Plan: copy and paste manually. The active route cannot provide a verified, interactive replacement. For long configuration text, use `wayexpand new` to generate a file.".into()
            }
            (Language::English, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Plan: paced synthetic keystrokes, estimated {predicted_ms} ms; compatibility mode is enabled, replacement is non-atomic, and the application result remains unverified.")
            }
            (Language::German, Strategy::VerifiedReplacement) => {
                "Plan: atomaren Routenvertrag verwenden. Die Einfügung muss noch in einer echten Anwendung getestet werden.".into()
            }
            (Language::German, Strategy::NativeTextCommit) => {
                "Plan: native Texteingabe. Der Kompatibilitätsmodus ist aktiv; die Ersetzung ist nicht atomar und das Anwendungsergebnis bleibt unverifiziert.".into()
            }
            (Language::German, Strategy::ClipboardAssistedPaste) => {
                "Plan: manuell kopieren und einfügen. Der aktive Weg bietet keine verifizierte interaktive Ersetzung.".into()
            }
            (Language::German, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Plan: simulierte Tastenanschläge, geschätzt {predicted_ms} ms; der Kompatibilitätsmodus ist aktiv, die Ersetzung ist nicht atomar und das Anwendungsergebnis bleibt unverifiziert.")
            }
            (Language::Indonesian, Strategy::VerifiedReplacement) => {
                "Rencana: gunakan kontrak rute atomik. Penyisipan pada aplikasi tetap perlu diuji di aplikasi nyata.".into()
            }
            (Language::Indonesian, Strategy::NativeTextCommit) => {
                "Rencana: komit teks native. Mode kompatibilitas aktif; penggantian non-atomik dan hasil aplikasi belum terverifikasi.".into()
            }
            (Language::Indonesian, Strategy::ClipboardAssistedPaste) => {
                "Rencana: salin dan tempel secara manual. Jalur aktif tidak menyediakan penggantian interaktif terverifikasi.".into()
            }
            (Language::Indonesian, Strategy::PacedSyntheticKeystrokes { predicted_ms }) => {
                format!("Rencana: ketukan papan ketik sintetis, perkiraan {predicted_ms} ms; mode kompatibilitas aktif, penggantian non-atomik, dan hasil aplikasi belum terverifikasi.")
            }
        }
    }

    pub fn replacement_pill(&self, assurance: ReplacementAssurance) -> &'static str {
        match (self.lang, assurance) {
            (Language::English, ReplacementAssurance::Verified) => {
                "Replacement: Atomic route contract"
            }
            (Language::English, ReplacementAssurance::FocusTrackedBestEffort) => {
                "Replacement: Focus-tracked best effort"
            }
            (Language::English, ReplacementAssurance::Unverified) => "Replacement: Unverified",
            (Language::English, ReplacementAssurance::Blocked) => "Replacement: Blocked",
            (Language::English, ReplacementAssurance::Unavailable) => "Replacement: Unavailable",
            (Language::German, ReplacementAssurance::Verified) => {
                "Ersetzung: Atomarer Routenvertrag"
            }
            (Language::German, ReplacementAssurance::FocusTrackedBestEffort) => {
                "Ersetzung: Fokusgeprüft, bestmöglich"
            }
            (Language::German, ReplacementAssurance::Unverified) => "Ersetzung: Unverifiziert",
            (Language::German, ReplacementAssurance::Blocked) => "Ersetzung: Blockiert",
            (Language::German, ReplacementAssurance::Unavailable) => "Ersetzung: Nicht verfügbar",
            (Language::Indonesian, ReplacementAssurance::Verified) => {
                "Penggantian: kontrak rute atomik"
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
            (Language::English, ReplacementAssurance::Verified) => "The active route advertises an atomic replacement contract, so trigger deletion and replacement are submitted as one operation. This route-level guarantee does not confirm that this specific application rendered the expected text; recent output acknowledgements remain unverified until an application test confirms the result.",
            (Language::English, ReplacementAssurance::FocusTrackedBestEffort) => "The window tracker confirms focus immediately before typing, but the route cannot verify that the text at the cursor is still the trigger; deletion and insertion are separate, non-atomic steps. Physical typing can interleave, delete surrounding text, corrupt nearby text, or leave partial output. Already-delivered text cannot be rolled back, and WayExpand never retries uncertain output. This route has no accepted physical-session target-integrity certification. A detected focus change stops output.",
            (Language::English, ReplacementAssurance::Unverified) => "This explicitly allowed keystroke route cannot verify the target field, focus, or trigger text it erases; deletion and insertion are separate, non-atomic steps. Physical typing can interleave, delete surrounding text, corrupt nearby text, or leave partial output. Already-delivered text cannot be rolled back, and WayExpand never retries uncertain output. This route has no accepted physical-session target-integrity certification. Strict mode refuses this operation.",
            (Language::English, ReplacementAssurance::Blocked) => "Strict mode refuses keystroke replacement because the route cannot verify what it erases. Choose a verified route or explicitly allow a compatibility mode in Settings → Engine.",
            (Language::English, ReplacementAssurance::Unavailable) => "No output route is connected.",
            (Language::German, ReplacementAssurance::Verified) => "Der aktive Weg bietet einen atomaren Ersetzungsvertrag: Löschen des Auslösers und Einfügen des Ersatzes werden als ein Vorgang übermittelt. Diese Routengarantie bestätigt nicht, dass diese konkrete Anwendung den erwarteten Text dargestellt hat; Ausgabe-Bestätigungen bleiben unverifiziert, bis ein Anwendungstest das Ergebnis bestätigt.",
            (Language::German, ReplacementAssurance::FocusTrackedBestEffort) => "Der Fenster-Tracker bestätigt den Fokus direkt vor dem Tippen, aber der Weg kann nicht prüfen, ob am Cursor noch der Auslöser steht; Löschen und Einfügen sind getrennte, nicht atomare Schritte. Physische Eingaben können sich überlagern, umliegenden Text löschen oder beschädigen oder eine unvollständige Ausgabe hinterlassen. Bereits gesendeter Text kann nicht zurückgenommen werden; WayExpand wiederholt unsichere Ausgaben nie. Für diesen Weg gibt es keine anerkannte Integritätszertifizierung in einer realen Desktop-Sitzung. Ein erkannter Fokuswechsel stoppt die Ausgabe.",
            (Language::German, ReplacementAssurance::Unverified) => "Dieser ausdrücklich erlaubte Tastenanschlag-Weg kann Zielfeld, Fokus oder Auslösertext nicht verifizieren; Löschen und Einfügen sind getrennte, nicht atomare Schritte. Physische Eingaben können sich überlagern, umliegenden Text löschen oder beschädigen oder eine unvollständige Ausgabe hinterlassen. Bereits gesendeter Text kann nicht zurückgenommen werden; WayExpand wiederholt unsichere Ausgaben nie. Für diesen Weg gibt es keine anerkannte Integritätszertifizierung in einer realen Desktop-Sitzung. Der strikte Modus verweigert diesen Vorgang.",
            (Language::German, ReplacementAssurance::Blocked) => "Der strikte Modus verweigert die Ersetzung per Tastenanschlag, weil der Weg den gelöschten Text nicht prüfen kann. Wählen Sie einen verifizierten Weg oder erlauben Sie ausdrücklich einen Kompatibilitätsmodus unter Einstellungen → Engine.",
            (Language::German, ReplacementAssurance::Unavailable) => "Kein Ausgabeweg ist verbunden.",
            (Language::Indonesian, ReplacementAssurance::Verified) => "Jalur aktif menawarkan kontrak penggantian atomik: penghapusan pemicu dan pengiriman pengganti dilakukan sebagai satu operasi. Jaminan tingkat rute ini tidak memastikan bahwa aplikasi ini menampilkan teks yang diharapkan; pengakuan keluaran tetap belum terverifikasi sampai pengujian aplikasi mengonfirmasi hasilnya.",
            (Language::Indonesian, ReplacementAssurance::FocusTrackedBestEffort) => "Pelacak jendela memastikan fokus tepat sebelum mengetik, tetapi jalur ini tidak dapat memastikan teks di kursor masih berupa pemicu; penghapusan dan penyisipan adalah langkah terpisah yang tidak atomik. Ketikan fisik dapat bercampur, menghapus teks di sekitarnya, merusak teks, atau meninggalkan keluaran yang tidak lengkap. Teks yang sudah terkirim tidak dapat dibatalkan, dan WayExpand tidak pernah mengulang keluaran yang hasilnya tidak pasti. Jalur ini belum memiliki sertifikasi integritas target dari sesi desktop fisik yang diterima. Perubahan fokus yang terdeteksi akan menghentikan keluaran.",
            (Language::Indonesian, ReplacementAssurance::Unverified) => "Jalur tombol yang diizinkan secara eksplisit ini tidak dapat memastikan kolom tujuan, fokus, atau teks pemicu yang dihapus; penghapusan dan penyisipan adalah langkah terpisah yang tidak atomik. Ketikan fisik dapat bercampur, menghapus teks di sekitarnya, merusak teks, atau meninggalkan keluaran yang tidak lengkap. Teks yang sudah terkirim tidak dapat dibatalkan, dan WayExpand tidak pernah mengulang keluaran yang hasilnya tidak pasti. Jalur ini belum memiliki sertifikasi integritas target dari sesi desktop fisik yang diterima. Mode ketat menolak operasi ini.",
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

    pub fn keystroke_limit_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Characters (1–5000; default 8). For non-atomic replacement, this limit applies only after you enable compatibility consent. Higher values allow longer text but increase the time an insertion can be interrupted and leave partial text; libei keysym output is about 83 characters/second. Keep it short for everyday use; use Quick Picker or Generate File for long text."
            }
            Language::German => {
                "Zeichen (1–5000; Standard 8). Bei nicht-atomaren Ersetzungen gilt dieses Limit erst nach ausdrücklicher Kompatibilitätsfreigabe. Höhere Werte erlauben längere Texte, erhöhen aber die Zeit, in der eine Ausgabe unterbrochen werden und unvollständigen Text hinterlassen kann; libei-Keysym-Ausgabe schafft etwa 83 Zeichen/Sekunde. Für den Alltag kurz halten; für lange Texte Quick Picker oder Dateierstellung verwenden."
            }
            Language::Indonesian => {
                "Karakter (1–5000; bawaan 8). Untuk penggantian non-atomik, batas ini berlaku hanya setelah izin kompatibilitas diaktifkan. Nilai lebih tinggi mengizinkan teks lebih panjang tetapi memperbesar waktu penyisipan dapat terhenti dan meninggalkan teks parsial; keluaran keysym libei sekitar 83 karakter/detik. Gunakan nilai pendek sehari-hari; pakai Quick Picker atau Buat File untuk teks panjang."
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

    pub fn terminal_paste_option(&self, mode: TerminalPaste) -> &'static str {
        match (self.lang, mode) {
            (Language::English, TerminalPaste::Off) => "Off: type",
            (Language::English, TerminalPaste::Multiline) => "Multi-line and long text",
            (Language::English, TerminalPaste::Always) => "Always",
            (Language::German, TerminalPaste::Off) => "Aus: tippen",
            (Language::German, TerminalPaste::Multiline) => "Mehrzeiliger und langer Text",
            (Language::German, TerminalPaste::Always) => "Immer",
            (Language::Indonesian, TerminalPaste::Off) => "Nonaktif: ketik",
            (Language::Indonesian, TerminalPaste::Multiline) => "Teks multibaris dan panjang",
            (Language::Indonesian, TerminalPaste::Always) => "Selalu",
        }
    }

    pub fn terminal_paste_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "In your terminal apps, the libei route can paste a replacement with the clipboard and Ctrl+Shift+V instead of typing it. Typed line breaks press Enter, so a shell runs each line as it arrives; a paste arrives at once and bracketed paste waits for your Enter. Pasted text is not limited by the keystroke limit or the keyboard layout. The clipboard manager is asked not to keep the replacement, and your previous clipboard text is restored. If the clipboard holds something other than text, the replacement is typed."
            }
            Language::German => {
                "In Ihren Terminal-Apps kann der libei-Weg eine Ersetzung über die Zwischenablage und Strg+Umschalt+V einfügen, statt sie zu tippen. Getippte Zeilenumbrüche drücken die Eingabetaste, sodass eine Shell jede Zeile sofort ausführt; eingefügter Text kommt auf einmal an, und Bracketed Paste wartet auf Ihre Eingabetaste. Eingefügter Text ist weder durch das Tastenanschlag-Limit noch durch das Tastaturlayout begrenzt. Die Zwischenablageverwaltung wird gebeten, die Ersetzung nicht zu speichern, und Ihr vorheriger Text in der Zwischenablage wird wiederhergestellt. Enthält die Zwischenablage etwas anderes als Text, wird die Ersetzung getippt."
            }
            Language::Indonesian => {
                "Di aplikasi terminal Anda, jalur libei dapat menempelkan pengganti melalui papan klip dan Ctrl+Shift+V alih-alih mengetiknya. Baris baru yang diketik menekan Enter, sehingga shell menjalankan setiap baris begitu tiba; tempelan tiba sekaligus dan bracketed paste menunggu Enter Anda. Teks yang ditempel tidak dibatasi oleh batas penekanan tombol maupun tata letak keyboard. Pengelola papan klip diminta untuk tidak menyimpan pengganti, dan teks papan klip Anda sebelumnya dipulihkan. Jika papan klip berisi selain teks, pengganti diketik."
            }
        }
    }
}
