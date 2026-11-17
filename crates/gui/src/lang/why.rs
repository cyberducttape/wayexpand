//! "Why didn't my trigger expand?" (see `wayexpand_core::ExpansionDiagnosis`).

use super::{Language, Strings};
use wayexpand_core::{MatchingBlock as B, OutcomeKind as K, Verdict as V};

impl Strings {
    pub(crate) fn why_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Why didn't my trigger expand?",
            Language::German => "Warum wurde mein Trigger nicht ersetzt?",
            Language::Indonesian => "Mengapa pemicu saya tidak diperluas?",
        }
    }

    pub(crate) fn why_intro(&self) -> &'static str {
        match self.lang {
            Language::English => "Typing route (keyboard service), from its last two minutes. No typed text is recorded.",
            Language::German => "Eingabeweg (Tastaturdienst), aus seinen letzten zwei Minuten. Getippter Text wird nicht gespeichert.",
            Language::Indonesian => "Jalur pengetikan (layanan keyboard), dari dua menit terakhirnya. Teks yang diketik tidak dicatat.",
        }
    }

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
            (V::NothingMatched, En) => ("Typing arrives, but nothing has expanded in the last two minutes. Usually the text did not match: a typo, a word-boundary trigger typed right after a letter, or an app filter that leaves the app out.", Some("Type exactly what you typed into the matcher preview to check it.")),
            (V::NothingMatched, De) => ("Eingaben kommen an, aber in den letzten zwei Minuten wurde nichts ersetzt. Meist passte der Text nicht: ein Tippfehler, ein Wortgrenzen-Trigger direkt nach einem Buchstaben oder ein App-Filter, der die App ausschließt.", Some("Tippen Sie denselben Text in die Matcher-Vorschau, um ihn zu prüfen.")),
            (V::NothingMatched, Id) => ("Ketikan masuk, tetapi tidak ada yang diperluas dalam dua menit terakhir. Biasanya teksnya tidak cocok: salah ketik, pemicu batas kata yang diketik tepat setelah huruf, atau filter aplikasi yang mengecualikan aplikasinya.", Some("Ketik teks yang sama di pratinjau pencocok untuk memeriksanya.")),
            (V::LastStopped(kind), lang) => self.why_stopped(kind, lang),
            (V::LastExpanded, En) => ("The route accepted the last expansion. If the text did not appear, the app ignored or changed it; WayExpand cannot see the app's text.", None),
            (V::LastExpanded, De) => ("Der Eingabeweg hat die letzte Ersetzung angenommen. Fehlt der Text, hat die App ihn ignoriert oder verändert; WayExpand sieht den Text der App nicht.", None),
            (V::LastExpanded, Id) => ("Jalur menerima perluasan terakhir. Jika teksnya tidak muncul, aplikasi mengabaikan atau mengubahnya; WayExpand tidak dapat melihat teks aplikasi.", None),
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
            (K::NotApplied, En) => ("The route refused the last expansion before changing anything.", None),
            (K::NotApplied, De) => ("Der Eingabeweg hat die letzte Ersetzung abgelehnt, bevor er etwas änderte.", None),
            (K::NotApplied, Id) => ("Jalur menolak perluasan terakhir sebelum mengubah apa pun.", None),
            (K::Unknown, En) => ("The route could not tell whether the app received the last expansion.", Some("Check the field before typing on.")),
            (K::Unknown, De) => ("Der Eingabeweg konnte nicht feststellen, ob die App die letzte Ersetzung erhielt.", Some("Prüfen Sie das Feld, bevor Sie weitertippen.")),
            (K::Unknown, Id) => ("Jalur tidak dapat memastikan apakah aplikasi menerima perluasan terakhir.", Some("Periksa kolomnya sebelum lanjut mengetik.")),
            (K::Partial, En) => ("The last expansion was only partly written.", Some("Check the field and correct it by hand.")),
            (K::Partial, De) => ("Die letzte Ersetzung wurde nur teilweise geschrieben.", Some("Prüfen Sie das Feld und korrigieren Sie es von Hand.")),
            (K::Partial, Id) => ("Perluasan terakhir hanya tertulis sebagian.", Some("Periksa kolomnya dan perbaiki secara manual.")),
            (K::BlockedByPolicy, En) => ("Organization or command policy stopped the last expansion.", None),
            (K::BlockedByPolicy, De) => ("Organisations- oder Befehlsrichtlinie hat die letzte Ersetzung gestoppt.", None),
            (K::BlockedByPolicy, Id) => ("Kebijakan organisasi atau perintah menghentikan perluasan terakhir.", None),
            (K::InterruptedByTyping, En) => ("More typing arrived before WayExpand could type the replacement, so it dropped the expansion rather than risk typing in the wrong place.", Some("Pause briefly after the trigger.")),
            (K::InterruptedByTyping, De) => ("Weitere Eingaben kamen, bevor WayExpand die Ersetzung tippen konnte; es verwarf sie, statt an der falschen Stelle zu tippen.", Some("Halten Sie nach dem Trigger kurz inne.")),
            (K::InterruptedByTyping, Id) => ("Ketikan lain masuk sebelum WayExpand sempat mengetik penggantinya, jadi perluasan dibatalkan agar tidak mengetik di tempat yang salah.", Some("Berhenti sejenak setelah pemicu.")),
            (K::SnippetActionFailed, En) => ("The command or form behind the last snippet failed.", Some("Open the snippet and check its command or form fields.")),
            (K::SnippetActionFailed, De) => ("Der Befehl oder das Formular hinter dem letzten Snippet ist fehlgeschlagen.", Some("Öffnen Sie das Snippet und prüfen Sie den Befehl oder die Formularfelder.")),
            (K::SnippetActionFailed, Id) => ("Perintah atau formulir di balik cuplikan terakhir gagal.", Some("Buka cuplikannya dan periksa perintah atau kolom formulirnya.")),
            (K::Expanded | K::ExpandedCursorMisplaced | K::Other, En) => ("The last expansion stopped; the details are below.", None),
            (K::Expanded | K::ExpandedCursorMisplaced | K::Other, De) => ("Die letzte Ersetzung wurde abgebrochen; Details stehen unten.", None),
            (K::Expanded | K::ExpandedCursorMisplaced | K::Other, Id) => ("Perluasan terakhir berhenti; detailnya ada di bawah.", None),
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
            .chain([V::NoTypingSeen, V::NothingMatched, V::LastExpanded]);
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
