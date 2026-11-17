//! Quick Picker messages (see `picker::PickerMessage`).

use super::{Language, Strings};
use crate::picker::{
    CopyReason, CopyReason as C, PickerMessage as M, PickerReadiness, PickerReadiness as R,
};
use wayexpand_core::InsertFailure as F;

impl Strings {
    /// The short header chip for what the Quick Picker does now.
    pub(crate) fn picker_readiness_chip(&self, readiness: PickerReadiness) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (readiness, self.lang) {
            (R::TypesDirectly, En) => "Quick Picker: types",
            (R::TypesDirectly, De) => "Schnellwähler: tippt",
            (R::TypesDirectly, Id) => "Picker: mengetik",
            (R::Copies(_), En) => "Quick Picker: copies",
            (R::Copies(_), De) => "Schnellwähler: kopiert",
            (R::Copies(_), Id) => "Picker: menyalin",
            (R::Unavailable(_), En) => "Quick Picker: off",
            (R::Unavailable(_), De) => "Schnellwähler: aus",
            (R::Unavailable(_), Id) => "Picker: mati",
        }
    }

    /// One sentence on what the Quick Picker does now, and why.
    pub(crate) fn picker_readiness_detail(&self, readiness: PickerReadiness) -> String {
        use Language::{English as En, German as De, Indonesian as Id};
        let reason = |reason: CopyReason| match (reason, self.lang) {
            (C::ServiceOff, En) => "the WayExpand service is not running",
            (C::ServiceOff, De) => "der WayExpand-Dienst läuft nicht",
            (C::ServiceOff, Id) => "layanan WayExpand tidak berjalan",
            (C::NotAtomic, En) => "this typing route cannot replace text in one step",
            (C::NotAtomic, De) => "dieser Eingabeweg kann Text nicht in einem Schritt ersetzen",
            (C::NotAtomic, Id) => {
                "jalur pengetikan ini tidak dapat mengganti teks dalam satu langkah"
            }
            (C::NoWindowIdentity, En) => {
                "WayExpand cannot tell exactly which window you were using"
            }
            (C::NoWindowIdentity, De) => {
                "WayExpand kann das zuletzt benutzte Fenster nicht genau bestimmen"
            }
            (C::NoWindowIdentity, Id) => {
                "WayExpand tidak dapat memastikan jendela yang sedang Anda pakai"
            }
        };
        match (readiness, self.lang) {
            (R::TypesDirectly, En) => "Quick Picker: Enter types the snippet into the app you were using.".into(),
            (R::TypesDirectly, De) => "Schnellwähler: Enter tippt das Snippet in die zuletzt benutzte App.".into(),
            (R::TypesDirectly, Id) => "Picker: Enter mengetik cuplikan ke aplikasi yang sedang Anda pakai.".into(),
            (R::Copies(why), En) => format!("Quick Picker: Enter copies the snippet for you to paste, because {}.", reason(why)),
            (R::Copies(why), De) => format!("Schnellwähler: Enter kopiert das Snippet zum Einfügen, weil {}.", reason(why)),
            (R::Copies(why), Id) => format!("Picker: Enter menyalin cuplikan untuk Anda tempel, karena {}.", reason(why)),
            (R::Unavailable(why), En) => format!("Quick Picker: cannot insert here, because {} and your organization's policy does not allow copying.", reason(why)),
            (R::Unavailable(why), De) => format!("Schnellwähler: kann hier nicht einfügen, weil {} und die Richtlinie Ihrer Organisation kein Kopieren erlaubt.", reason(why)),
            (R::Unavailable(why), Id) => format!("Picker: tidak dapat menyisipkan di sini, karena {} dan kebijakan organisasi Anda tidak mengizinkan penyalinan.", reason(why)),
        }
    }

    /// Names what the main status chip is about, now that the Quick Picker
    /// has its own.
    pub(crate) fn typed_triggers_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Typed triggers",
            Language::German => "Getippte Trigger",
            Language::Indonesian => "Pemicu yang diketik",
        }
    }

    pub(crate) fn picker_show_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Show snippets that can't be used here",
            Language::German => "Snippets anzeigen, die hier nicht nutzbar sind",
            Language::Indonesian => "Tampilkan cuplikan yang tidak bisa dipakai di sini",
        }
    }

    pub(crate) fn picker_too_late_to_cancel(&self) -> &'static str {
        match self.lang {
            Language::English => "Too late to cancel: WayExpand already has this insert. The picker shows the result when it arrives.",
            Language::German => "Abbrechen ist nicht mehr möglich: WayExpand hat diese Einfügung bereits. Der Schnellwähler zeigt das Ergebnis, sobald es vorliegt.",
            Language::Indonesian => "Terlambat untuk membatalkan: WayExpand sudah menerima penyisipan ini. Picker menampilkan hasilnya begitu tersedia.",
        }
    }

    pub(crate) fn picker_message(&self, message: &M) -> String {
        use Language::{English as En, German as De, Indonesian as Id};
        match (message, self.lang) {
            (M::FocusUnknown, En) => "Could not identify the window you came from, so nothing was inserted.".into(),
            (M::FocusUnknown, De) => "Das Ausgangsfenster ließ sich nicht bestimmen, deshalb wurde nichts eingefügt.".into(),
            (M::FocusUnknown, Id) => "Jendela asal tidak dapat dikenali, jadi tidak ada yang disisipkan.".into(),
            (M::WorkerStartFailed(error), En) => format!("Could not start inserting: {error}"),
            (M::WorkerStartFailed(error), De) => format!("Das Einfügen konnte nicht starten: {error}"),
            (M::WorkerStartFailed(error), Id) => format!("Penyisipan tidak dapat dimulai: {error}"),
            (M::WorkerStopped, En) => "Inserting stopped before WayExpand confirmed the result. Check the field before copying.".into(),
            (M::WorkerStopped, De) => "Das Einfügen endete, bevor WayExpand das Ergebnis bestätigte. Prüfen Sie das Feld, bevor Sie kopieren.".into(),
            (M::WorkerStopped, Id) => "Penyisipan berhenti sebelum WayExpand memastikan hasilnya. Periksa kolom sebelum menyalin.".into(),
            (M::RenderFailed(error), En) => format!("Could not prepare this snippet: {error}"),
            (M::RenderFailed(error), De) => format!("Dieses Snippet ließ sich nicht vorbereiten: {error}"),
            (M::RenderFailed(error), Id) => format!("Cuplikan ini tidak dapat disiapkan: {error}"),
            (M::CopyNotAllowedByPolicy, En) => "Your organization's policy does not allow copying snippets for pasting.".into(),
            (M::CopyNotAllowedByPolicy, De) => "Die Richtlinie Ihrer Organisation erlaubt kein Kopieren von Snippets zum Einfügen.".into(),
            (M::CopyNotAllowedByPolicy, Id) => "Kebijakan organisasi Anda tidak mengizinkan menyalin cuplikan untuk ditempel.".into(),
            (M::CopyNotAllowedForSnippet, En) => "This snippet cannot be copied: it is limited to specific apps or reads the clipboard, and a paste cannot check where it goes.".into(),
            (M::CopyNotAllowedForSnippet, De) => "Dieses Snippet kann nicht kopiert werden: Es ist auf bestimmte Apps beschränkt oder liest die Zwischenablage, und beim Einfügen lässt sich das Ziel nicht prüfen.".into(),
            (M::CopyNotAllowedForSnippet, Id) => "Cuplikan ini tidak bisa disalin: terbatas untuk aplikasi tertentu atau membaca papan klip, dan penempelan tidak bisa memeriksa tujuannya.".into(),
            (M::SnippetGone, En) => "This snippet is no longer in your library.".into(),
            (M::SnippetGone, De) => "Dieses Snippet ist nicht mehr in Ihrer Bibliothek.".into(),
            (M::SnippetGone, Id) => "Cuplikan ini sudah tidak ada di pustaka Anda.".into(),
            (M::OlderService, En) => "The running WayExpand service is older than this picker and cannot confirm inserts, so nothing was inserted. Restart it with `wayexpand service restart --yes`, or copy the snippet instead.".into(),
            (M::OlderService, De) => "Der laufende WayExpand-Dienst ist älter als dieser Schnellwähler und kann Einfügungen nicht bestätigen, deshalb wurde nichts eingefügt. Starten Sie ihn mit `wayexpand service restart --yes` neu oder kopieren Sie das Snippet.".into(),
            (M::OlderService, Id) => "Layanan WayExpand yang berjalan lebih lama dari picker ini dan tidak dapat memastikan penyisipan, jadi tidak ada yang disisipkan. Mulai ulang dengan `wayexpand service restart --yes`, atau salin cuplikannya.".into(),
            (M::CallFailed(error), En) => format!("Could not reach WayExpand, so nothing was inserted: {error}"),
            (M::CallFailed(error), De) => format!("WayExpand war nicht erreichbar, deshalb wurde nichts eingefügt: {error}"),
            (M::CallFailed(error), Id) => format!("WayExpand tidak dapat dihubungi, jadi tidak ada yang disisipkan: {error}"),
            (M::Refused(reply), En) => format!("WayExpand refused the insert: {reply}"),
            (M::Refused(reply), De) => format!("WayExpand hat die Einfügung abgelehnt: {reply}"),
            (M::Refused(reply), Id) => format!("WayExpand menolak penyisipan: {reply}"),
            (M::FocusNotReturned, En) => "Focus did not return to the window you came from, so nothing was inserted.".into(),
            (M::FocusNotReturned, De) => "Der Fokus kehrte nicht zum Ausgangsfenster zurück, deshalb wurde nichts eingefügt.".into(),
            (M::FocusNotReturned, Id) => "Fokus tidak kembali ke jendela asal, jadi tidak ada yang disisipkan.".into(),
            (M::LostContact(error), En) => format!("Lost contact with WayExpand before the insert was confirmed: {error}. Check the field before copying."),
            (M::LostContact(error), De) => format!("Die Verbindung zu WayExpand brach ab, bevor die Einfügung bestätigt war: {error}. Prüfen Sie das Feld, bevor Sie kopieren."),
            (M::LostContact(error), Id) => format!("Kontak dengan WayExpand terputus sebelum penyisipan dipastikan: {error}. Periksa kolom sebelum menyalin."),
            (M::UnexpectedReply(reply), En) => format!("Unexpected reply from WayExpand: {reply}. Check the field before copying."),
            (M::UnexpectedReply(reply), De) => format!("Unerwartete Antwort von WayExpand: {reply}. Prüfen Sie das Feld, bevor Sie kopieren."),
            (M::UnexpectedReply(reply), Id) => format!("Balasan tak terduga dari WayExpand: {reply}. Periksa kolom sebelum menyalin."),
            (M::NotConfirmed(seconds), En) => format!("WayExpand did not confirm the insert within {seconds} seconds. Check the field before copying, in case part of the snippet was typed."),
            (M::NotConfirmed(seconds), De) => format!("WayExpand hat die Einfügung nicht innerhalb von {seconds} Sekunden bestätigt. Prüfen Sie das Feld, bevor Sie kopieren, falls ein Teil getippt wurde."),
            (M::NotConfirmed(seconds), Id) => format!("WayExpand tidak memastikan penyisipan dalam {seconds} detik. Periksa kolom sebelum menyalin, kalau-kalau sebagian cuplikan sudah diketik."),
            (M::LostTrack, En) => "WayExpand lost track of this insert, perhaps because it restarted. Check the field before copying.".into(),
            (M::LostTrack, De) => "WayExpand hat diese Einfügung aus den Augen verloren, vielleicht durch einen Neustart. Prüfen Sie das Feld, bevor Sie kopieren.".into(),
            (M::LostTrack, Id) => "WayExpand kehilangan jejak penyisipan ini, mungkin karena dimulai ulang. Periksa kolom sebelum menyalin.".into(),
            (M::Uncertain, En) => "The desktop could not confirm the insert, so part of the snippet may have been typed. Check the field before copying.".into(),
            (M::Uncertain, De) => "Der Desktop konnte die Einfügung nicht bestätigen; ein Teil des Snippets wurde eventuell getippt. Prüfen Sie das Feld, bevor Sie kopieren.".into(),
            (M::Uncertain, Id) => "Desktop tidak dapat memastikan penyisipan, jadi sebagian cuplikan mungkin sudah diketik. Periksa kolom sebelum menyalin.".into(),
            (M::NotInserted(failure), lang) => {
                let reason = match (failure, lang) {
                    (F::FocusChanged, En) => "Focus moved to another window before the snippet was typed.",
                    (F::FocusChanged, De) => "Der Fokus wechselte in ein anderes Fenster, bevor das Snippet getippt wurde.",
                    (F::FocusChanged, Id) => "Fokus berpindah ke jendela lain sebelum cuplikan diketik.",
                    (F::PolicyRefused, En) => "Your organization's policy blocks this snippet.",
                    (F::PolicyRefused, De) => "Die Richtlinie Ihrer Organisation blockiert dieses Snippet.",
                    (F::PolicyRefused, Id) => "Kebijakan organisasi Anda memblokir cuplikan ini.",
                    (F::InputArrived, En) => "Typing started before the snippet could be inserted.",
                    (F::InputArrived, De) => "Es wurde getippt, bevor das Snippet eingefügt werden konnte.",
                    (F::InputArrived, Id) => "Pengetikan dimulai sebelum cuplikan sempat disisipkan.",
                    (F::NoBackend, En) => "WayExpand is reconnecting to the desktop.",
                    (F::NoBackend, De) => "WayExpand verbindet sich neu mit dem Desktop.",
                    (F::NoBackend, Id) => "WayExpand sedang menyambung ulang ke desktop.",
                    (F::Refused, En) => "WayExpand could not use this snippet; it may be disabled or changed.",
                    (F::Refused, De) => "WayExpand konnte dieses Snippet nicht verwenden; es ist eventuell deaktiviert oder geändert.",
                    (F::Refused, Id) => "WayExpand tidak dapat memakai cuplikan ini; mungkin dinonaktifkan atau diubah.",
                    (F::NotApplied, En) => "The desktop did not accept the text.",
                    (F::NotApplied, De) => "Der Desktop hat den Text nicht angenommen.",
                    (F::NotApplied, Id) => "Desktop tidak menerima teksnya.",
                    (F::Superseded, En) => "A newer insert replaced this one.",
                    (F::Superseded, De) => "Eine neuere Einfügung hat diese ersetzt.",
                    (F::Superseded, Id) => "Penyisipan yang lebih baru menggantikan yang ini.",
                };
                let nothing = match lang {
                    En => "Nothing was inserted.",
                    De => "Es wurde nichts eingefügt.",
                    Id => "Tidak ada yang disisipkan.",
                };
                format!("{reason} {nothing}")
            }
        }
    }
}
