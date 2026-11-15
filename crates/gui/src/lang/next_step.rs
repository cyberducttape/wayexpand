//! One plain-language problem and one action (see `app::next_step`).

use super::{Language, Strings};
use crate::app::{NextAction as Action, Problem};

impl Strings {
    pub fn problem_text(&self, problem: Problem) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (problem, self.lang) {
            (Problem::ServiceOff, En) => "WayExpand is off, so snippets do not expand.",
            (Problem::ServiceOff, De) => "WayExpand ist aus, deshalb werden Snippets nicht ersetzt.",
            (Problem::ServiceOff, Id) => "WayExpand mati, jadi cuplikan tidak diperluas.",
            (Problem::Paused, En) => "Expansion is paused.",
            (Problem::Paused, De) => "Die Ersetzung ist pausiert.",
            (Problem::Paused, Id) => "Ekspansi dijeda.",
            (Problem::PermissionNeeded, En) => "Your desktop has not yet allowed WayExpand to read and type text.",
            (Problem::PermissionNeeded, De) => "Ihr Desktop hat WayExpand noch nicht erlaubt, Text zu lesen und zu tippen.",
            (Problem::PermissionNeeded, Id) => "Desktop Anda belum mengizinkan WayExpand membaca dan mengetik teks.",
            (Problem::PermissionRevoked, En) => "Your desktop withdrew WayExpand's permission to type.",
            (Problem::PermissionRevoked, De) => "Ihr Desktop hat WayExpand die Erlaubnis zum Tippen entzogen.",
            (Problem::PermissionRevoked, Id) => "Desktop Anda mencabut izin WayExpand untuk mengetik.",
            (Problem::RestartNeeded, En) => "The running WayExpand service is from an older version.",
            (Problem::RestartNeeded, De) => "Der laufende WayExpand-Dienst stammt aus einer älteren Version.",
            (Problem::RestartNeeded, Id) => "Layanan WayExpand yang berjalan berasal dari versi lama.",
            (Problem::BuildUnknown, En) => "The running WayExpand service cannot say which version it is.",
            (Problem::BuildUnknown, De) => "Der laufende WayExpand-Dienst kann seine Version nicht angeben.",
            (Problem::BuildUnknown, Id) => "Layanan WayExpand yang berjalan tidak dapat menyebutkan versinya.",
            (Problem::InputSourceNotSetUp, En) => "WayExpand is not set up as a GNOME input source yet.",
            (Problem::InputSourceNotSetUp, De) => "WayExpand ist noch nicht als GNOME-Eingabequelle eingerichtet.",
            (Problem::InputSourceNotSetUp, Id) => "WayExpand belum disiapkan sebagai sumber masukan GNOME.",
            (Problem::InputSourceNotSelected, En) => "WayExpand is not the selected GNOME input source.",
            (Problem::InputSourceNotSelected, De) => "WayExpand ist nicht die ausgewählte GNOME-Eingabequelle.",
            (Problem::InputSourceNotSelected, Id) => "WayExpand bukan sumber masukan GNOME yang dipilih.",
            (Problem::ReplacementNotAllowed, En) => "Replacing typed text on this desktop needs your approval first.",
            (Problem::ReplacementNotAllowed, De) => "Das Ersetzen getippten Texts braucht auf diesem Desktop zuerst Ihre Zustimmung.",
            (Problem::ReplacementNotAllowed, Id) => "Mengganti teks yang diketik di desktop ini perlu persetujuan Anda dulu.",
            (Problem::Connecting, En) => "WayExpand is connecting to your desktop.",
            (Problem::Connecting, De) => "WayExpand verbindet sich mit Ihrem Desktop.",
            (Problem::Connecting, Id) => "WayExpand sedang terhubung ke desktop Anda.",
            (Problem::RouteFailed, En) => "WayExpand lost its connection to your desktop's typing.",
            (Problem::RouteFailed, De) => "WayExpand hat die Verbindung zur Texteingabe Ihres Desktops verloren.",
            (Problem::RouteFailed, Id) => "WayExpand kehilangan koneksi ke pengetikan desktop Anda.",
            (Problem::Unsupported, En) => "This desktop does not let WayExpand replace typed text. You can still insert snippets with the Quick Picker.",
            (Problem::Unsupported, De) => "Dieser Desktop erlaubt WayExpand nicht, getippten Text zu ersetzen. Snippets lassen sich weiter mit dem Schnellwähler einfügen.",
            (Problem::Unsupported, Id) => "Desktop ini tidak mengizinkan WayExpand mengganti teks yang diketik. Anda tetap bisa menyisipkan cuplikan dengan Quick Picker.",
            (Problem::FieldCannotReplace, En) => "The app you typed in last does not let WayExpand replace text there. Use the Quick Picker in that app.",
            (Problem::FieldCannotReplace, De) => "Die zuletzt benutzte App lässt WayExpand dort keinen Text ersetzen. Nutzen Sie in dieser App den Schnellwähler.",
            (Problem::FieldCannotReplace, Id) => "Aplikasi tempat Anda terakhir mengetik tidak mengizinkan WayExpand mengganti teks di sana. Gunakan Quick Picker di aplikasi itu.",
        }
    }

    pub fn action_label(&self, action: Action) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (action, self.lang) {
            (Action::TurnOn, En) => "Turn on",
            (Action::TurnOn, De) => "Einschalten",
            (Action::TurnOn, Id) => "Aktifkan",
            (Action::Resume, En) => "Resume",
            (Action::Resume, De) => "Fortsetzen",
            (Action::Resume, Id) => "Lanjutkan",
            (Action::RunSetup, En) => "Fix with guided setup",
            (Action::RunSetup, De) => "Mit geführter Einrichtung beheben",
            (Action::RunSetup, Id) => "Perbaiki dengan penyiapan terpandu",
            (Action::Restart, En) => "Restart WayExpand",
            (Action::Restart, De) => "WayExpand neu starten",
            (Action::Restart, Id) => "Mulai ulang WayExpand",
            (Action::SelectInputSource, En) => "Select WayExpand",
            (Action::SelectInputSource, De) => "WayExpand auswählen",
            (Action::SelectInputSource, Id) => "Pilih WayExpand",
            (Action::ReviewConsent, En) => "Review and allow",
            (Action::ReviewConsent, De) => "Prüfen und erlauben",
            (Action::ReviewConsent, Id) => "Tinjau dan izinkan",
            (Action::CheckAgain, En) => "Check again",
            (Action::CheckAgain, De) => "Erneut prüfen",
            (Action::CheckAgain, Id) => "Periksa lagi",
            (Action::QuickPicker, En) => "Open Quick Picker",
            (Action::QuickPicker, De) => "Schnellwähler öffnen",
            (Action::QuickPicker, Id) => "Buka Quick Picker",
        }
    }

    pub fn next_step_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Details",
            Language::German => "Details",
            Language::Indonesian => "Detail",
        }
    }
}
