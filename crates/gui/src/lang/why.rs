//! "Why didn't my trigger expand?" (see `wayexpand_core::ExpansionDiagnosis`).

use super::{Language, Strings};
use wayexpand_core::{MatchingBlock as B, OutcomeKind as K, Verdict as V};

impl Strings {
    /// The answer, and what to do about it if anything.
    pub(crate) fn why_verdict(
        &self,
        verdict: V,
        terminal_only: bool,
    ) -> (&'static str, Option<&'static str>) {
        use Language::{English as En, German as De, Indonesian as Id};
        match (verdict, self.lang) {
            (V::MatchingOff(block), lang) => (self.why_block(block, lang), Some(self.why_block_fix(block, lang))),
            (V::NoTypingSeen, En) if terminal_only => ("No typing has reached the service since it started. It only handles your terminal apps.", Some("Type the trigger in a terminal app.")),
            (V::NoTypingSeen, De) if terminal_only => ("Seit dem Start hat den Dienst keine Eingabe erreicht. Er bedient nur Ihre Terminal-Apps.", Some("Tippen Sie den Trigger in einer Terminal-App.")),
            (V::NoTypingSeen, Id) if terminal_only => ("Belum ada ketikan yang sampai ke layanan sejak dimulai. Layanan ini hanya menangani aplikasi terminal Anda.", Some("Ketik pemicunya di aplikasi terminal.")),
            (V::NoTypingSeen, En) => ("No typing has reached the service since it started.", Some("Check the input route in the diagnostics above.")),
            (V::NoTypingSeen, De) => ("Seit dem Start hat den Dienst keine Eingabe erreicht.", Some("Prüfen Sie den Eingabeweg in der Diagnose oben.")),
            (V::NoTypingSeen, Id) => ("Belum ada ketikan yang sampai ke layanan sejak dimulai.", Some("Periksa jalur input di diagnostik di atas.")),
            (V::NothingMatched, En) => ("Typing arrives, but nothing has expanded in the last two minutes. The trigger may not match, or an app filter may exclude this app.", Some("Use Matcher preview to check the pattern. A match there is not a desktop test; then run the real-application typing test from Getting started.")),
            (V::NothingMatched, De) => ("Eingaben kommen an, aber in den letzten zwei Minuten wurde nichts ersetzt. Der Auslöser passt möglicherweise nicht oder ein App-Filter schließt diese App aus.", Some("Prüfen Sie das Muster in der Matcher-Vorschau. Ein Treffer dort ist kein Desktop-Test; führen Sie danach den echten Anwendungstest unter „Erste Schritte“ aus.")),
            (V::NothingMatched, Id) => ("Ketikan masuk, tetapi tidak ada yang diperluas dalam dua menit terakhir. Pemicu mungkin tidak cocok atau filter aplikasi mengecualikan aplikasi ini.", Some("Periksa pola di Pratinjau pencocok. Kecocokan di sana bukan uji desktop; lalu jalankan uji pengetikan aplikasi nyata dari Memulai.")),
            (V::InputStatusUnknown, En) => ("The daemon did not report input activity, so WayExpand cannot tell whether the affected field sent typing to it.", Some("Run Debug → Live, type harmless text in the affected app, and check whether input is observed. If it is not, open Diagnostics. Use Quick Picker to copy and paste meanwhile.")),
            (V::InputStatusUnknown, De) => ("Der Dienst meldet keine Eingabeaktivität. WayExpand kann daher nicht feststellen, ob das betroffene Feld Eingaben weitergeleitet hat.", Some("Starten Sie Diagnose → Live, tippen Sie harmlosen Text in der betroffenen App und prüfen Sie, ob Eingaben beobachtet werden. Öffnen Sie andernfalls die Diagnose. Bis dahin können Sie mit dem Schnellwähler kopieren und einfügen.")),
            (V::InputStatusUnknown, Id) => ("Layanan tidak melaporkan aktivitas input, jadi WayExpand belum tahu apakah kolom tersebut mengirim ketikan.", Some("Jalankan Debug → Live, ketik teks aman di aplikasi terkait, lalu periksa apakah input terdeteksi. Jika tidak, buka Diagnostik. Sementara itu, gunakan Quick Picker untuk menyalin dan menempel.")),
            (V::LastStopped(kind), lang) => self.why_stopped(kind, lang),
            (V::LastExpanded, En) => ("WayExpand matched the trigger and submitted replacement output, but cannot verify that the application changed its text.", Some("Check the field. If the replacement is missing or wrong, run the real-application typing test from Getting started; do not retry uncertain output.")),
            (V::LastExpanded, De) => ("WayExpand hat die letzte Ersetzung erkannt und gesendet, kann aber nicht prüfen, ob die Anwendung ihren Text geändert hat.", Some("Prüfen Sie das Feld. Fehlt der Text oder ist er falsch, führen Sie den echten Anwendungstest unter „Erste Schritte“ aus; unsichere Ausgaben nicht wiederholen.")),
            (V::LastExpanded, Id) => ("WayExpand mencocokkan dan mengirim perluasan terakhir, tetapi tidak dapat memastikan bahwa aplikasi mengubah teksnya.", Some("Periksa kolomnya. Jika hasilnya hilang atau salah, jalankan uji pengetikan aplikasi nyata dari Memulai; jangan ulangi keluaran yang belum pasti.")),
        }
    }

    fn why_block(&self, block: B, lang: Language) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (block, lang) {
            (B::CaptureDisconnected, En) => "Keyboard capture is not connected, so nothing you type reaches WayExpand.",
            (B::CaptureDisconnected, De) => "Die Tastaturerfassung ist nicht verbunden, daher erreicht WayExpand nichts, was Sie tippen.",
            (B::CaptureDisconnected, Id) => "Penangkapan keyboard tidak terhubung, jadi tidak ada ketikan yang sampai ke WayExpand.",
            (B::KeyboardLayoutUnavailable, En) => "WayExpand cannot read your keyboard layout, so it cannot tell which characters you typed.",
            (B::KeyboardLayoutUnavailable, De) => "WayExpand kann Ihr Tastaturlayout nicht lesen und weiß daher nicht, welche Zeichen Sie getippt haben.",
            (B::KeyboardLayoutUnavailable, Id) => "WayExpand tidak dapat membaca tata letak keyboard Anda, jadi tidak tahu karakter apa yang Anda ketik.",
            (B::OutputUnavailable, En) => "No output route is connected, so WayExpand cannot replace a trigger.",
            (B::OutputUnavailable, De) => "Kein Ausgabeweg ist verbunden, daher kann WayExpand keinen Trigger ersetzen.",
            (B::OutputUnavailable, Id) => "Tidak ada jalur keluaran yang terhubung, jadi WayExpand tidak dapat mengganti pemicu.",
            (B::PolicyRoute, En) => "Your organization's policy does not allow this typing route.",
            (B::PolicyRoute, De) => "Die Richtlinie Ihrer Organisation erlaubt diesen Eingabeweg nicht.",
            (B::PolicyRoute, Id) => "Kebijakan organisasi Anda tidak mengizinkan jalur pengetikan ini.",
            (B::Paused, En) => "WayExpand is paused.",
            (B::Paused, De) => "WayExpand ist pausiert.",
            (B::Paused, Id) => "WayExpand sedang dijeda.",
            (B::ConfigurationUnhealthy, En) => "The service could not load the configuration, so it has no usable snippets.",
            (B::ConfigurationUnhealthy, De) => "Der Dienst konnte die Konfiguration nicht laden und hat daher keine nutzbaren Snippets.",
            (B::ConfigurationUnhealthy, Id) => "Layanan tidak dapat memuat konfigurasi, jadi tidak ada cuplikan yang bisa dipakai.",
            (B::ReplacementNotAllowed, En) => "This route types keystrokes and has not been allowed to replace typed text.",
            (B::ReplacementNotAllowed, De) => "Dieser Weg tippt Tastenanschläge und darf getippten Text noch nicht ersetzen.",
            (B::ReplacementNotAllowed, Id) => "Jalur ini mengetik penekanan tombol dan belum diizinkan mengganti teks yang diketik.",
            (B::Other, En) => "Matching is off for a reason this version does not know.",
            (B::Other, De) => "Das Matching ist aus einem Grund aus, den diese Version nicht kennt.",
            (B::Other, Id) => "Pencocokan mati karena alasan yang tidak dikenal versi ini.",
        }
    }

    fn why_block_fix(&self, block: B, lang: Language) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (block, lang) {
            (B::Paused, En) => "Choose Resume in the ☰ menu.",
            (B::Paused, De) => "Wählen Sie „Fortsetzen“ im ☰-Menü.",
            (B::Paused, Id) => "Pilih Lanjutkan di menu ☰.",
            (B::ConfigurationUnhealthy, En) => "Fix the error shown in the library, then save.",
            (B::ConfigurationUnhealthy, De) => {
                "Beheben Sie den in der Bibliothek angezeigten Fehler und speichern Sie."
            }
            (B::ConfigurationUnhealthy, Id) => {
                "Perbaiki galat yang ditampilkan di pustaka, lalu simpan."
            }
            (_, En) => "Follow the next step shown at the top of this window.",
            (_, De) => "Folgen Sie dem nächsten Schritt oben in diesem Fenster.",
            (_, Id) => "Ikuti langkah berikutnya yang ditampilkan di bagian atas jendela ini.",
        }
    }

    fn why_stopped(&self, kind: K, lang: Language) -> (&'static str, Option<&'static str>) {
        use Language::{English as En, German as De, Indonesian as Id};
        match (kind, lang) {
            (K::NotApplied, En) => ("The route refused the last expansion before changing anything.", Some("Follow the corrective action shown above, then try one harmless real-application test.")),
            (K::NotApplied, De) => ("Der Eingabeweg hat die letzte Ersetzung abgelehnt, bevor er etwas änderte.", Some("Folgen Sie der obigen Korrektur und testen Sie anschließend einmal in einer unkritischen Anwendung.")),
            (K::NotApplied, Id) => ("Jalur menolak perluasan terakhir sebelum mengubah apa pun.", Some("Ikuti tindakan perbaikan di atas, lalu uji sekali di aplikasi nyata dengan teks yang tidak sensitif.")),
            (K::Unknown, En) => ("The route could not tell whether the app received the last expansion.", Some("Check the field before typing on.")),
            (K::Unknown, De) => ("Der Eingabeweg konnte nicht feststellen, ob die App die letzte Ersetzung erhielt.", Some("Prüfen Sie das Feld, bevor Sie weitertippen.")),
            (K::Unknown, Id) => ("Jalur tidak dapat memastikan apakah aplikasi menerima perluasan terakhir.", Some("Periksa kolomnya sebelum lanjut mengetik.")),
            (K::Partial, En) => ("The last expansion was only partly written.", Some("Check the field and correct it by hand.")),
            (K::Partial, De) => ("Die letzte Ersetzung wurde nur teilweise geschrieben.", Some("Prüfen Sie das Feld und korrigieren Sie es von Hand.")),
            (K::Partial, Id) => ("Perluasan terakhir hanya tertulis sebagian.", Some("Periksa kolomnya dan perbaiki secara manual.")),
            (K::BlockedByPolicy, En) => ("Organization or command policy stopped the last expansion.", Some("Open Diagnostics → Policy to identify the refusing rule; ask the policy administrator to change it if appropriate.")),
            (K::BlockedByPolicy, De) => ("Organisations- oder Befehlsrichtlinie hat die letzte Ersetzung gestoppt.", Some("Öffnen Sie Diagnose → Richtlinie, um die Regel zu finden; bitten Sie bei Bedarf die zuständige Administration um eine Änderung.")),
            (K::BlockedByPolicy, Id) => ("Kebijakan organisasi atau perintah menghentikan perluasan terakhir.", Some("Buka Diagnostik → Kebijakan untuk melihat aturan yang menolak; minta administrator kebijakan mengubahnya jika sesuai.")),
            (K::InterruptedByTyping, En) => ("More typing arrived before WayExpand could type the replacement, so it dropped the expansion rather than risk typing in the wrong place.", Some("Pause briefly after the trigger.")),
            (K::InterruptedByTyping, De) => ("Weitere Eingaben kamen, bevor WayExpand die Ersetzung tippen konnte; es verwarf sie, statt an der falschen Stelle zu tippen.", Some("Halten Sie nach dem Trigger kurz inne.")),
            (K::InterruptedByTyping, Id) => ("Ketikan lain masuk sebelum WayExpand sempat mengetik penggantinya, jadi perluasan dibatalkan agar tidak mengetik di tempat yang salah.", Some("Berhenti sejenak setelah pemicu.")),
            (K::SnippetActionFailed, En) => ("The command or form behind the last snippet failed.", Some("Open the snippet and check its command or form fields.")),
            (K::SnippetActionFailed, De) => ("Der Befehl oder das Formular hinter dem letzten Snippet ist fehlgeschlagen.", Some("Öffnen Sie das Snippet und prüfen Sie den Befehl oder die Formularfelder.")),
            (K::SnippetActionFailed, Id) => ("Perintah atau formulir di balik cuplikan terakhir gagal.", Some("Buka cuplikannya dan periksa perintah atau kolom formulirnya.")),
            (K::Expanded | K::Other, En) => ("WayExpand submitted output, but the application result is unverified.", Some("Inspect the field and run a real-application typing test if the text is missing; do not replay uncertain output.")),
            (K::Expanded | K::Other, De) => ("WayExpand hat die Ausgabe gesendet, aber das Ergebnis in der Anwendung ist ungeprüft.", Some("Prüfen Sie das Feld und testen Sie in einer echten Anwendung, falls Text fehlt; unsichere Ausgabe nicht erneut senden.")),
            (K::Expanded | K::Other, Id) => ("WayExpand mengirim keluaran, tetapi hasil di aplikasi belum diverifikasi.", Some("Periksa kolomnya dan uji di aplikasi nyata jika teks hilang; jangan kirim ulang keluaran yang belum pasti.")),
            (K::ExpandedCursorMisplaced, En) => ("WayExpand submitted the replacement but could not confirm cursor placement; application text is unverified.", Some("Inspect the field and cursor before continuing. Run a real-application test if the result is wrong.")),
            (K::ExpandedCursorMisplaced, De) => ("WayExpand hat die Ersetzung gesendet, konnte die Cursorposition aber nicht bestätigen; der Anwendungstext ist ungeprüft.", Some("Prüfen Sie Feld und Cursor, bevor Sie weitertippen. Testen Sie in einer echten Anwendung, wenn das Ergebnis falsch ist.")),
            (K::ExpandedCursorMisplaced, Id) => ("WayExpand mengirim penggantian tetapi tidak dapat memastikan posisi kursor; teks aplikasi belum diverifikasi.", Some("Periksa kolom dan kursor sebelum melanjutkan. Uji di aplikasi nyata jika hasilnya salah.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verdict_is_answered_in_every_language() {
        let blocks = [
            B::CaptureDisconnected,
            B::KeyboardLayoutUnavailable,
            B::OutputUnavailable,
            B::PolicyRoute,
            B::Paused,
            B::ConfigurationUnhealthy,
            B::ReplacementNotAllowed,
            B::Other,
        ];
        let kinds = [
            K::NotApplied,
            K::Unknown,
            K::Partial,
            K::BlockedByPolicy,
            K::InterruptedByTyping,
            K::SnippetActionFailed,
            K::Other,
        ];
        let verdicts = blocks
            .into_iter()
            .map(V::MatchingOff)
            .chain(kinds.into_iter().map(V::LastStopped))
            .chain([
                V::NoTypingSeen,
                V::InputStatusUnknown,
                V::NothingMatched,
                V::LastExpanded,
            ]);
        for verdict in verdicts {
            for terminal_only in [false, true] {
                let answers = [Language::English, Language::German, Language::Indonesian]
                    .map(|lang| Strings::new(lang).why_verdict(verdict, terminal_only));
                for (answer, fix) in answers {
                    assert!(answer.ends_with('.'), "{verdict:?}: {answer}");
                    assert!(fix.is_none_or(|fix| fix.ends_with('.')), "{verdict:?}");
                }
                // Each language says it in its own words.
                assert_ne!(answers[0].0, answers[1].0, "{verdict:?}");
                assert_ne!(answers[0].0, answers[2].0, "{verdict:?}");
            }
        }
    }
}
