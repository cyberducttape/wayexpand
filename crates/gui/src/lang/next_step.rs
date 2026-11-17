//! One plain-language problem and one action (see `app::next_step`).

use super::{Language, Strings};
use crate::app::{NextAction as Action, Problem};

impl Strings {
    pub fn problem_text(&self, problem: Problem) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (problem, self.lang) {
            (Problem::ServiceOff, En) => "WayExpand is off, so snippets do not expand.",
            (Problem::ServiceOff, De) => {
                "WayExpand ist aus, deshalb werden Snippets nicht ersetzt."
            }
            (Problem::ServiceOff, Id) => "WayExpand mati, jadi cuplikan tidak diperluas.",
            (Problem::NoTypingRoute, En) => {
                "Only the Quick Picker and hotkeys are on: nothing reads what you type yet."
            }
            (Problem::NoTypingRoute, De) => {
                "Nur Schnellwähler und Tastenkürzel sind aktiv: Noch liest nichts mit, was Sie tippen."
            }
            (Problem::NoTypingRoute, Id) => {
                "Hanya Picker dan pintasan yang aktif: belum ada yang membaca ketikan Anda."
            }
            (Problem::Paused, En) => "Expansion is paused.",
            (Problem::Paused, De) => "Die Ersetzung ist pausiert.",
            (Problem::Paused, Id) => "Ekspansi dijeda.",
            (Problem::PermissionNeeded, En) => {
                "Your desktop has not yet allowed WayExpand to read and type text."
            }
            (Problem::PermissionNeeded, De) => {
                "Ihr Desktop hat WayExpand noch nicht erlaubt, Text zu lesen und zu tippen."
            }
            (Problem::PermissionNeeded, Id) => {
                "Desktop Anda belum mengizinkan WayExpand membaca dan mengetik teks."
            }
            (Problem::PermissionRevoked, En) => {
                "Your desktop withdrew WayExpand's permission to type."
            }
            (Problem::PermissionRevoked, De) => {
                "Ihr Desktop hat WayExpand die Erlaubnis zum Tippen entzogen."
            }
            (Problem::PermissionRevoked, Id) => {
                "Desktop Anda mencabut izin WayExpand untuk mengetik."
            }
            (Problem::RestartNeeded, En) => {
                "The running WayExpand service is from an older version."
            }
            (Problem::RestartNeeded, De) => {
                "Der laufende WayExpand-Dienst stammt aus einer älteren Version."
            }
            (Problem::RestartNeeded, Id) => {
                "Layanan WayExpand yang berjalan berasal dari versi lama."
            }
            (Problem::BuildUnknown, En) => {
                "The running WayExpand service cannot say which version it is."
            }
            (Problem::BuildUnknown, De) => {
                "Der laufende WayExpand-Dienst kann seine Version nicht angeben."
            }
            (Problem::BuildUnknown, Id) => {
                "Layanan WayExpand yang berjalan tidak dapat menyebutkan versinya."
            }
            (Problem::InputSourceNotSetUp, En) => {
                "WayExpand is not set up as a GNOME input source yet."
            }
            (Problem::InputSourceNotSetUp, De) => {
                "WayExpand ist noch nicht als GNOME-Eingabequelle eingerichtet."
            }
            (Problem::InputSourceNotSetUp, Id) => {
                "WayExpand belum disiapkan sebagai sumber masukan GNOME."
            }
            (Problem::IbusEngineNotRegistered, En) => {
                "GNOME has a WayExpand input source, but IBus cannot find its engine."
            }
            (Problem::IbusEngineNotRegistered, De) => {
                "GNOME hat eine WayExpand-Eingabequelle, aber IBus findet die Engine nicht."
            }
            (Problem::IbusEngineNotRegistered, Id) => {
                "GNOME memiliki sumber input WayExpand, tetapi IBus tidak menemukan mesinnya."
            }
            (Problem::InputSourceNotSelected, En) => "WayExpand is not the selected input method.",
            (Problem::InputSourceNotSelected, De) => {
                "WayExpand ist nicht als Eingabemethode ausgewählt."
            }
            (Problem::InputSourceNotSelected, Id) => "WayExpand bukan metode masukan yang dipilih.",
            (Problem::InputStatusPending, En) => {
                "WayExpand cannot confirm the current input-method setup yet."
            }
            (Problem::InputStatusPending, De) => {
                "WayExpand kann die aktuelle Einrichtung der Eingabemethode noch nicht bestätigen."
            }
            (Problem::InputStatusPending, Id) => {
                "WayExpand belum dapat memastikan penyiapan metode input saat ini."
            }
            (Problem::IbusEngineUnavailable, En) => {
                "The selected WayExpand input source has no running engine."
            }
            (Problem::IbusEngineUnavailable, De) => {
                "Für die ausgewählte WayExpand-Eingabequelle läuft keine Engine."
            }
            (Problem::IbusEngineUnavailable, Id) => {
                "Sumber masukan WayExpand yang dipilih tidak memiliki mesin aktif."
            }
            (Problem::InputStatusStale, En) => "GNOME input status changed or is out of date.",
            (Problem::InputStatusStale, De) => {
                "Der GNOME-Eingabestatus hat sich geändert oder ist veraltet."
            }
            (Problem::InputStatusStale, Id) => {
                "Status masukan GNOME berubah atau sudah kedaluwarsa."
            }
            (Problem::ReplacementNotAllowed, En) => {
                "Replacing typed text on this desktop needs your approval first."
            }
            (Problem::ReplacementNotAllowed, De) => {
                "Das Ersetzen getippten Texts braucht auf diesem Desktop zuerst Ihre Zustimmung."
            }
            (Problem::ReplacementNotAllowed, Id) => {
                "Mengganti teks yang diketik di desktop ini perlu persetujuan Anda dulu."
            }
            (Problem::ReplacementConsentUnknown, En) => {
                "WayExpand could not confirm whether replacement is allowed."
            }
            (Problem::ReplacementConsentUnknown, De) => {
                "WayExpand konnte nicht bestätigen, ob Ersetzungen erlaubt sind."
            }
            (Problem::ReplacementConsentUnknown, Id) => {
                "WayExpand tidak dapat memastikan apakah penggantian diizinkan."
            }
            (Problem::Connecting, En) => "WayExpand is connecting to your desktop.",
            (Problem::Connecting, De) => "WayExpand verbindet sich mit Ihrem Desktop.",
            (Problem::Connecting, Id) => "WayExpand sedang terhubung ke desktop Anda.",
            (Problem::RouteFailed, En) => "WayExpand lost its connection to your desktop's typing.",
            (Problem::RouteFailed, De) => {
                "WayExpand hat die Verbindung zur Texteingabe Ihres Desktops verloren."
            }
            (Problem::RouteFailed, Id) => {
                "WayExpand kehilangan koneksi ke pengetikan desktop Anda."
            }
            (Problem::Unsupported, En) => {
                "This desktop does not let WayExpand replace typed text. You can still insert snippets with the Quick Picker."
            }
            (Problem::Unsupported, De) => {
                "Dieser Desktop erlaubt WayExpand nicht, getippten Text zu ersetzen. Snippets lassen sich weiter mit dem Schnellwähler einfügen."
            }
            (Problem::Unsupported, Id) => {
                "Desktop ini tidak mengizinkan WayExpand mengganti teks yang diketik. Anda tetap bisa menyisipkan cuplikan dengan Quick Picker."
            }
            (Problem::FieldCannotReplace, En) => {
                "The app you typed in last does not let WayExpand replace text there. Use the Quick Picker in that app."
            }
            (Problem::FieldCannotReplace, De) => {
                "Die zuletzt benutzte App lässt WayExpand dort keinen Text ersetzen. Nutzen Sie in dieser App den Schnellwähler."
            }
            (Problem::FieldCannotReplace, Id) => {
                "Aplikasi tempat Anda terakhir mengetik tidak mengizinkan WayExpand mengganti teks di sana. Gunakan Quick Picker di aplikasi itu."
            }
            (Problem::ApplicationDeliveryUnverified, En) => {
                "The route is connected, but WayExpand has not verified that replacement text appears in an application."
            }
            (Problem::ApplicationDeliveryUnverified, De) => {
                "Die Verbindung steht, aber WayExpand hat nicht geprüft, ob Ersetzungstext in einer Anwendung erscheint."
            }
            (Problem::ApplicationDeliveryUnverified, Id) => {
                "Rute terhubung, tetapi WayExpand belum memverifikasi teks pengganti muncul di aplikasi."
            }
            (Problem::OutputUnverified, En) => {
                "WayExpand recognized the trigger and sent replacement output, but the application’s text was not verified."
            }
            (Problem::OutputUnverified, De) => {
                "WayExpand hat den Trigger erkannt und Ersatztext gesendet, aber der Anwendungstext wurde nicht geprüft."
            }
            (Problem::OutputUnverified, Id) => {
                "WayExpand mengenali pemicu dan mengirim teks pengganti, tetapi teks di aplikasi belum diverifikasi."
            }
            (Problem::PartialOutput, En) => {
                "WayExpand reports that only part of the replacement may have been written."
            }
            (Problem::PartialOutput, De) => {
                "WayExpand meldet, dass möglicherweise nur ein Teil der Ersetzung geschrieben wurde."
            }
            (Problem::PartialOutput, Id) => {
                "WayExpand melaporkan bahwa hanya sebagian teks pengganti yang mungkin tertulis."
            }
            (Problem::OutputNotApplied, En) => "WayExpand refused the most recent expansion before it changed the field.",
            (Problem::OutputNotApplied, De) => "WayExpand hat die letzte Ersetzung abgelehnt, bevor das Feld geändert wurde.",
            (Problem::OutputNotApplied, Id) => "WayExpand menolak ekspansi terakhir sebelum mengubah kolom.",
            (Problem::OutputBlockedByPolicy, En) => "A policy stopped the most recent expansion before output was sent.",
            (Problem::OutputBlockedByPolicy, De) => "Eine Richtlinie hat die letzte Ersetzung gestoppt, bevor Text gesendet wurde.",
            (Problem::OutputBlockedByPolicy, Id) => "Kebijakan menghentikan ekspansi terakhir sebelum teks dikirim.",
            (Problem::OutputInterrupted, En) => "WayExpand stopped the most recent expansion because more typing arrived.",
            (Problem::OutputInterrupted, De) => "WayExpand hat die letzte Ersetzung abgebrochen, weil weitere Eingaben ankamen.",
            (Problem::OutputInterrupted, Id) => "WayExpand menghentikan ekspansi terakhir karena ada ketikan tambahan.",
            (Problem::SnippetActionFailed, En) => "The most recent snippet's form or command failed before output was delivered.",
            (Problem::SnippetActionFailed, De) => "Das Formular oder der Befehl des letzten Snippets ist vor der Ausgabe fehlgeschlagen.",
            (Problem::SnippetActionFailed, Id) => "Formulir atau perintah cuplikan terakhir gagal sebelum keluaran dikirim.",
            (Problem::OutputStatusUnknown, En) => "WayExpand recorded a recent output result that this version cannot classify.",
            (Problem::OutputStatusUnknown, De) => "WayExpand hat ein aktuelles Ausgabeergebnis aufgezeichnet, das diese Version nicht einordnen kann.",
            (Problem::OutputStatusUnknown, Id) => "WayExpand mencatat hasil keluaran terbaru yang tidak dapat diklasifikasikan versi ini.",
            (Problem::StatusUnknown, En) => {
                "WayExpand could not read the current service status, so it cannot tell whether typing is reaching it."
            }
            (Problem::StatusUnknown, De) => {
                "WayExpand konnte den aktuellen Dienststatus nicht lesen und kann daher nicht feststellen, ob Eingaben ankommen."
            }
            (Problem::StatusUnknown, Id) => {
                "WayExpand tidak dapat membaca status layanan saat ini, jadi belum dapat memastikan apakah ketikan diterima."
            }
            (Problem::RouteStatusUnknown, En) => {
                "The service is running, but the active typing route could not be identified."
            }
            (Problem::RouteStatusUnknown, De) => {
                "Der Dienst läuft, aber der aktive Eingabeweg konnte nicht ermittelt werden."
            }
            (Problem::RouteStatusUnknown, Id) => {
                "Layanan berjalan, tetapi rute pengetikan aktif tidak dapat diketahui."
            }
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
            (Action::TestInApplication, En) => "Test in an app",
            (Action::TestInApplication, De) => "In einer App testen",
            (Action::TestInApplication, Id) => "Uji di aplikasi",
            (Action::InspectText, En) => "Refresh diagnostics",
            (Action::InspectText, De) => "Diagnose aktualisieren",
            (Action::InspectText, Id) => "Perbarui diagnostik",
            (Action::ReviewSnippet, En) => "Review snippet",
            (Action::ReviewSnippet, De) => "Snippet prüfen",
            (Action::ReviewSnippet, Id) => "Tinjau cuplikan",
        }
    }

    pub fn next_step_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Details",
            Language::German => "Details",
            Language::Indonesian => "Detail",
        }
    }

    /// The concrete follow-up for the one prioritized diagnosis. Keep this
    /// separate from the button label: some problems need a user action in
    /// the desktop before the button's live test can be meaningful.
    pub fn next_step_instruction(&self, problem: Problem, gnome_ibus: bool) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (problem, self.lang) {
            (Problem::InputStatusPending, En) => {
                "Refresh this check. If it remains unknown, open Diagnostics → IBus to see which registration, selection, engine, or consent check needs attention. Quick Picker can copy a snippet for manual paste meanwhile."
            }
            (Problem::InputStatusPending, De) => {
                "Aktualisieren Sie diese Prüfung. Bleibt der Status unbekannt, öffnen Sie Diagnose → IBus, um zu sehen, ob Registrierung, Auswahl, Engine oder Zustimmung betroffen ist. Bis dahin kann der Schnellwähler ein Snippet zum manuellen Einfügen kopieren."
            }
            (Problem::InputStatusPending, Id) => {
                "Perbarui pemeriksaan ini. Jika tetap tidak diketahui, buka Diagnostik → IBus untuk melihat pemeriksaan pendaftaran, pemilihan, mesin, atau izin yang perlu ditangani. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel manual."
            }
            (Problem::ReplacementConsentUnknown, En) => {
                "Open Diagnostics → IBus and run `wayexpand validate` to resolve the unreadable configuration or policy check. Refresh this diagnosis before enabling replacement."
            }
            (Problem::ReplacementConsentUnknown, De) => {
                "Öffnen Sie Diagnose → IBus und führen Sie `wayexpand validate` aus, um die nicht lesbare Konfigurations- oder Richtlinienprüfung zu beheben. Aktualisieren Sie die Diagnose, bevor Sie Ersetzungen erlauben."
            }
            (Problem::ReplacementConsentUnknown, Id) => {
                "Buka Diagnostik → IBus dan jalankan `wayexpand validate` untuk memperbaiki pemeriksaan konfigurasi atau kebijakan yang tidak terbaca. Perbarui diagnosis sebelum mengaktifkan penggantian."
            }
            (Problem::InputSourceNotSelected, En) if gnome_ibus => {
                "Text expansion is unavailable because GNOME has not selected WayExpand. Select it in Settings → Keyboard or the input-source menu, then run a live typing test. Quick Picker can copy a snippet for you to paste meanwhile."
            }
            (Problem::InputSourceNotSelected, De) if gnome_ibus => {
                "Texterweiterungen sind nicht verfügbar, weil GNOME WayExpand nicht ausgewählt hat. Wählen Sie es unter Einstellungen → Tastatur oder im Eingabequellen-Menü aus und führen Sie danach einen Live-Tipptest durch. Bis dahin können Sie ein Snippet mit dem Schnellwähler kopieren und einfügen."
            }
            (Problem::InputSourceNotSelected, Id) if gnome_ibus => {
                "Ekspansi teks tidak tersedia karena GNOME belum memilih WayExpand. Pilih di Pengaturan → Keyboard atau menu sumber input, lalu jalankan tes mengetik langsung. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel."
            }
            (Problem::IbusEngineNotRegistered, En) => {
                "Run `wayexpand ibus status --json` to confirm the missing engine registration, then run `wayexpand setup --backend ibus` and check the status again. Quick Picker can copy a snippet for manual paste meanwhile."
            }
            (Problem::IbusEngineNotRegistered, De) => {
                "Führen Sie `wayexpand ibus status --json` aus, um die fehlende Engine-Registrierung zu bestätigen. Starten Sie danach `wayexpand setup --backend ibus` und prüfen Sie den Status erneut. Bis dahin kann der Schnellwähler ein Snippet zum manuellen Einfügen kopieren."
            }
            (Problem::IbusEngineNotRegistered, Id) => {
                "Jalankan `wayexpand ibus status --json` untuk memastikan pendaftaran mesin belum ada, lalu jalankan `wayexpand setup --backend ibus` dan periksa status lagi. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel manual."
            }
            (Problem::InputSourceNotSelected, En) => {
                "Select WayExpand in your desktop's input-method settings, then run a live typing test. Quick Picker can copy a snippet for you to paste meanwhile."
            }
            (Problem::InputSourceNotSelected, De) => {
                "Wählen Sie WayExpand in den Eingabemethode-Einstellungen Ihres Desktops aus und führen Sie danach einen Live-Tipptest durch. Bis dahin können Sie ein Snippet mit dem Schnellwähler kopieren und einfügen."
            }
            (Problem::InputSourceNotSelected, Id) => {
                "Pilih WayExpand di pengaturan metode input desktop Anda, lalu jalankan tes mengetik langsung. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel."
            }
            (Problem::ApplicationDeliveryUnverified, En) => {
                "Run a real typing test in the affected app. Matcher preview checks only whether the rule matches; it does not prove that the app received the replacement."
            }
            (Problem::ApplicationDeliveryUnverified, De) => {
                "Führen Sie einen echten Tipptest in der betroffenen App durch. Die Matcher-Vorschau prüft nur die Regel, nicht ob die App den Ersatztext erhalten hat."
            }
            (Problem::ApplicationDeliveryUnverified, Id) => {
                "Jalankan tes mengetik sungguhan di aplikasi yang bermasalah. Pratinjau matcher hanya memeriksa kecocokan aturan, bukan apakah aplikasi menerima teks pengganti."
            }
            (Problem::OutputUnverified, En) => {
                "Inspect the target field and cursor now. Output was submitted but may not have appeared; do not retry it automatically. Then run one harmless real-application test."
            }
            (Problem::OutputUnverified, De) => {
                "Prüfen Sie jetzt das Zielfeld und den Cursor. Die Ausgabe wurde gesendet, ist aber möglicherweise nicht angekommen; nicht automatisch wiederholen. Führen Sie danach einen unkritischen Anwendungstest durch."
            }
            (Problem::OutputUnverified, Id) => {
                "Periksa kolom tujuan dan posisi kursor sekarang. Keluaran telah dikirim tetapi mungkin belum muncul; jangan ulangi otomatis. Lalu jalankan satu tes aman di aplikasi nyata."
            }
            (Problem::PartialOutput, En) => {
                "Inspect the target field before typing anything else. Correct partial text manually; WayExpand will not replay this insertion. Then run a harmless real-application test."
            }
            (Problem::PartialOutput, De) => {
                "Prüfen Sie das Zielfeld, bevor Sie weitertippen. Korrigieren Sie unvollständigen Text von Hand; WayExpand wiederholt diese Eingabe nicht. Führen Sie danach einen unkritischen Anwendungstest durch."
            }
            (Problem::PartialOutput, Id) => {
                "Periksa kolom tujuan sebelum mengetik lagi. Perbaiki teks yang tidak lengkap secara manual; WayExpand tidak akan mengulang penyisipan ini. Lalu jalankan tes aman di aplikasi nyata."
            }
            (Problem::OutputNotApplied, En) => "No replacement was sent; check whether the original trigger is still there. Refresh diagnostics and follow the specific output refusal.",
            (Problem::OutputNotApplied, De) => "Es wurde kein Ersatz gesendet; prüfen Sie, ob der ursprüngliche Trigger noch vorhanden ist. Aktualisieren Sie die Diagnose und folgen Sie dem konkreten Ablehnungsgrund.",
            (Problem::OutputNotApplied, Id) => "Teks pengganti tidak dikirim; periksa apakah pemicu asli masih ada. Perbarui diagnostik dan ikuti alasan penolakan keluaran yang ditampilkan.",
            (Problem::OutputBlockedByPolicy, En) => "Open Diagnostics → Policy to identify the refusing rule. Ask your policy administrator to change it if appropriate; use Quick Picker only if policy permits copying.",
            (Problem::OutputBlockedByPolicy, De) => "Öffnen Sie Diagnose → Richtlinie, um die ablehnende Regel zu finden. Bitten Sie gegebenenfalls die Richtlinienverwaltung um eine Änderung; nutzen Sie den Schnellwähler nur, wenn Kopieren erlaubt ist.",
            (Problem::OutputBlockedByPolicy, Id) => "Buka Diagnostik → Kebijakan untuk melihat aturan yang menolak. Minta administrator mengubahnya jika sesuai; gunakan Quick Picker hanya jika kebijakan mengizinkan penyalinan.",
            (Problem::OutputInterrupted, En) => "Inspect the target field before continuing. Pause briefly after the trigger, then run one harmless test in the affected app. If replacement text may have appeared, do not repeat it.",
            (Problem::OutputInterrupted, De) => "Prüfen Sie das Zielfeld, bevor Sie fortfahren. Warten Sie nach dem Trigger kurz und testen Sie dann einmal mit harmlosem Text in der betroffenen App. Falls Ersatztext erschienen sein könnte, wiederholen Sie ihn nicht.",
            (Problem::OutputInterrupted, Id) => "Periksa kolom tujuan sebelum melanjutkan. Berhenti sejenak setelah pemicu, lalu lakukan satu tes aman di aplikasi terkait. Jika teks pengganti mungkin sudah muncul, jangan ulangi.",
            (Problem::SnippetActionFailed, En) => "Open the snippet in the library and review its form fields or command result. WayExpand does not rerun a failed command automatically.",
            (Problem::SnippetActionFailed, De) => "Öffnen Sie das Snippet in der Bibliothek und prüfen Sie Formularfelder oder Befehlsausgabe. WayExpand führt einen fehlgeschlagenen Befehl nicht automatisch erneut aus.",
            (Problem::SnippetActionFailed, Id) => "Buka cuplikan di pustaka dan periksa isian formulir atau hasil perintahnya. WayExpand tidak menjalankan ulang perintah yang gagal secara otomatis.",
            (Problem::OutputStatusUnknown, En) => "Refresh diagnostics and inspect the latest outcome. Check the target field before retrying anything; use Quick Picker for copy and paste meanwhile.",
            (Problem::OutputStatusUnknown, De) => "Aktualisieren Sie die Diagnose und prüfen Sie das letzte Ergebnis. Kontrollieren Sie das Zielfeld, bevor Sie etwas wiederholen; nutzen Sie bis dahin den Schnellwähler zum Kopieren und Einfügen.",
            (Problem::OutputStatusUnknown, Id) => "Perbarui diagnostik dan periksa hasil terakhir. Periksa kolom tujuan sebelum mencoba lagi; sementara itu gunakan Quick Picker untuk salin dan tempel.",
            (Problem::StatusUnknown | Problem::RouteStatusUnknown, En) => {
                "Refresh this diagnosis. If status remains unavailable, open Diagnostics → Services and use Quick Picker in copy-only mode meanwhile."
            }
            (Problem::StatusUnknown | Problem::RouteStatusUnknown, De) => {
                "Aktualisieren Sie die Diagnose. Bleibt der Status unbekannt, öffnen Sie Diagnose → Dienste und verwenden Sie bis dahin den Schnellwähler im Kopiermodus."
            }
            (Problem::StatusUnknown | Problem::RouteStatusUnknown, Id) => {
                "Perbarui diagnosis ini. Jika status tetap tidak tersedia, buka Diagnostik → Layanan dan gunakan Quick Picker dalam mode salin saja sementara itu."
            }
            (Problem::FieldCannotReplace, En) => {
                "This field does not provide text WayExpand can safely replace. Open Quick Picker in copy-only mode and paste the snippet."
            }
            (Problem::FieldCannotReplace, De) => {
                "Dieses Feld stellt keinen Text bereit, den WayExpand sicher ersetzen kann. Öffnen Sie den Schnellwähler im Kopiermodus und fügen Sie das Snippet ein."
            }
            (Problem::FieldCannotReplace, Id) => {
                "Kolom ini tidak menyediakan teks yang dapat diganti WayExpand dengan aman. Buka Quick Picker dalam mode salin saja, lalu tempel cuplikannya."
            }
            (Problem::Unsupported, En) => {
                "This route cannot replace typed text here. Use Quick Picker in copy-only mode and paste the snippet."
            }
            (Problem::Unsupported, De) => {
                "Dieser Eingabeweg kann hier keinen getippten Text ersetzen. Nutzen Sie den Schnellwähler im Kopiermodus und fügen Sie das Snippet ein."
            }
            (Problem::Unsupported, Id) => {
                "Rute ini tidak dapat mengganti teks yang diketik di sini. Gunakan Quick Picker dalam mode salin saja, lalu tempel cuplikannya."
            }
            (Problem::NoTypingRoute, En) => {
                "Configure an input route with guided setup. Quick Picker remains available for copy and paste."
            }
            (Problem::NoTypingRoute, De) => {
                "Richten Sie mit der geführten Einrichtung einen Eingabeweg ein. Der Schnellwähler bleibt zum Kopieren und Einfügen verfügbar."
            }
            (Problem::NoTypingRoute, Id) => {
                "Siapkan rute input melalui penyiapan terpandu. Quick Picker tetap tersedia untuk salin dan tempel."
            }
            (_, En) => {
                "Follow the action shown here, then refresh this diagnosis. Quick Picker can copy a snippet for manual paste while expansion is unavailable."
            }
            (_, De) => {
                "Führen Sie die angezeigte Aktion aus und aktualisieren Sie danach diese Diagnose. Solange die Ersetzung nicht verfügbar ist, können Sie ein Snippet mit dem Schnellwähler kopieren und einfügen."
            }
            (_, Id) => {
                "Ikuti tindakan yang ditampilkan, lalu perbarui diagnosis ini. Quick Picker dapat menyalin cuplikan untuk ditempel secara manual selama ekspansi belum tersedia."
            }
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
            Language::English => {
                "Your confirmation could not be saved, so it counts for this session only; WayExpand asks again after a restart."
            }
            Language::German => {
                "Ihre Bestätigung konnte nicht gespeichert werden und gilt daher nur für diese Sitzung; WayExpand fragt nach einem Neustart erneut."
            }
            Language::Indonesian => {
                "Konfirmasi Anda tidak dapat disimpan, jadi hanya berlaku untuk sesi ini; WayExpand akan bertanya lagi setelah dimulai ulang."
            }
        }
    }
}
