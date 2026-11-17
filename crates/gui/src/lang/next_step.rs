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
            (Problem::NoTypingRoute, En) => "Only the Quick Picker and hotkeys are on: nothing reads what you type yet.",
            (Problem::NoTypingRoute, De) => "Nur Schnellwähler und Tastenkürzel sind aktiv: Noch liest nichts mit, was Sie tippen.",
            (Problem::NoTypingRoute, Id) => "Hanya Picker dan pintasan yang aktif: belum ada yang membaca ketikan Anda.",
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
            (Problem::InputSourceNotSelected, En) => "WayExpand is not the selected input method.",
            (Problem::InputSourceNotSelected, De) => "WayExpand ist nicht als Eingabemethode ausgewählt.",
            (Problem::InputSourceNotSelected, Id) => "WayExpand bukan metode masukan yang dipilih.",
            (Problem::InputStatusPending, En) => "WayExpand is checking whether the desktop selected its input method.",
            (Problem::InputStatusPending, De) => "WayExpand prüft, ob der Desktop die Eingabemethode ausgewählt hat.",
            (Problem::InputStatusPending, Id) => "WayExpand sedang memeriksa apakah desktop memilih metode masukannya.",
            (Problem::IbusEngineUnavailable, En) => "The selected WayExpand input source has no running engine.",
            (Problem::IbusEngineUnavailable, De) => "Für die ausgewählte WayExpand-Eingabequelle läuft keine Engine.",
            (Problem::IbusEngineUnavailable, Id) => "Sumber masukan WayExpand yang dipilih tidak memiliki mesin aktif.",
            (Problem::InputStatusStale, En) => "GNOME input status changed or is out of date.",
            (Problem::InputStatusStale, De) => "Der GNOME-Eingabestatus hat sich geändert oder ist veraltet.",
            (Problem::InputStatusStale, Id) => "Status masukan GNOME berubah atau sudah kedaluwarsa.",
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

impl Strings {
    pub fn last_confirmed(&self, application: &str, date: &str) -> String {
        match self.lang {
            Language::English => format!("Last confirmed working in {application} on {date}."),
            Language::German => {
                format!("Zuletzt am {date} in {application} als funktionierend bestätigt.")
            }
            Language::Indonesian => {
                format!("Terakhir dipastikan berfungsi di {application} pada {date}.")
            }
        }
    }

    pub fn setup_changed_since_test(&self, change: &str) -> String {
        match self.lang {
            Language::English => format!(
                "Your setup changed since the last confirmed test ({change}). Test it again from Getting started."
            ),
            Language::German => format!(
                "Ihre Einrichtung hat sich seit dem letzten bestätigten Test geändert ({change}). Testen Sie erneut über „Erste Schritte“."
            ),
            Language::Indonesian => format!(
                "Penyiapan Anda berubah sejak uji terakhir yang dipastikan ({change}). Uji lagi dari Memulai."
            ),
        }
    }

    pub fn setup_change(&self, change: &crate::setup_record::Staleness) -> String {
        use crate::setup_record::Staleness;
        match (change, self.lang) {
            (Staleness::RouteChanged { was }, Language::English) => {
                format!("the typing route was {was}")
            }
            (Staleness::RouteChanged { was }, Language::German) => {
                format!("der Eingabeweg war {was}")
            }
            (Staleness::RouteChanged { was }, Language::Indonesian) => {
                format!("jalur pengetikan sebelumnya {was}")
            }
            (Staleness::DesktopChanged { was }, Language::English) => {
                format!("the desktop was {was}")
            }
            (Staleness::DesktopChanged { was }, Language::German) => {
                format!("der Desktop war {was}")
            }
            (Staleness::DesktopChanged { was }, Language::Indonesian) => {
                format!("desktop sebelumnya {was}")
            }
            (Staleness::SessionChanged, Language::English) => {
                "you logged in again or the desktop restarted".into()
            }
            (Staleness::SessionChanged, Language::German) => {
                "Sie haben sich neu angemeldet oder der Desktop wurde neu gestartet".into()
            }
            (Staleness::SessionChanged, Language::Indonesian) => {
                "Anda masuk lagi atau desktop dimulai ulang".into()
            }
            (Staleness::BuildChanged { was }, Language::English) => {
                format!("WayExpand was updated from {was}")
            }
            (Staleness::BuildChanged { was }, Language::German) => {
                format!("WayExpand wurde von {was} aktualisiert")
            }
            (Staleness::BuildChanged { was }, Language::Indonesian) => {
                format!("WayExpand diperbarui dari {was}")
            }
            (Staleness::RouteRestarted, Language::English) => {
                "the input engine or service restarted".into()
            }
            (Staleness::RouteRestarted, Language::German) => {
                "die Eingabe-Engine oder der Dienst wurde neu gestartet".into()
            }
            (Staleness::RouteRestarted, Language::Indonesian) => {
                "mesin masukan atau layanan dimulai ulang".into()
            }
            (Staleness::Incomplete, Language::English) => {
                "the saved test does not say what it covered".into()
            }
            (Staleness::Incomplete, Language::German) => {
                "der gespeicherte Test sagt nicht, was er abdeckte".into()
            }
            (Staleness::Incomplete, Language::Indonesian) => {
                "uji tersimpan tidak menyebut apa yang dicakupnya".into()
            }
        }
    }
}

impl Strings {
    pub fn verification_not_saved(&self) -> &'static str {
        match self.lang {
            Language::English => "Your confirmation could not be saved, so it counts for this session only; WayExpand asks again after a restart.",
            Language::German => "Ihre Bestätigung konnte nicht gespeichert werden und gilt daher nur für diese Sitzung; WayExpand fragt nach einem Neustart erneut.",
            Language::Indonesian => "Konfirmasi Anda tidak dapat disimpan, jadi hanya berlaku untuk sesi ini; WayExpand akan bertanya lagi setelah dimulai ulang.",
        }
    }
}
