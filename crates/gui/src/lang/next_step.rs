//! One plain-language problem and one action (see `app::next_step`).

use super::{Language, Strings};
use crate::app::{NextAction as Action, Problem};

impl Strings {
    pub(crate) fn why_focus_scope_pattern(&self) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match self.lang {
            En => "Trigger match: blocked because focused-app identity is unknown.",
            De => "Trigger-Abgleich: gesperrt, weil die Identität der fokussierten Anwendung unbekannt ist.",
            Id => "Pencocokan pemicu: diblokir karena identitas aplikasi aktif tidak diketahui.",
        }
    }

    pub(crate) fn why_focus_scope_delivery(&self) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match self.lang {
            En => "Application text: no replacement output was attempted.",
            De => "Anwendungstext: Es wurde keine Ersetzung ausgegeben.",
            Id => "Teks aplikasi: tidak ada keluaran pengganti yang dicoba.",
        }
    }

    /// Evidence labels for a route that cannot currently deliver keystrokes.
    /// A prior real-app test or stale field event must not look like evidence
    /// that the currently blocked route is working.
    pub(crate) fn why_route_blocked_pattern(&self) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match self.lang {
            En => "Trigger match: not evaluated because the input route is not confirmed ready.",
            De => "Trigger-Abgleich: nicht geprüft, weil der Eingabeweg nicht als bereit bestätigt ist.",
            Id => "Pencocokan pemicu: belum diperiksa karena rute input belum dipastikan siap.",
        }
    }

    pub(crate) fn why_route_blocked_delivery(&self) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match self.lang {
            En => "Application text: this diagnosis has no replacement attempt to verify.",
            De => "Anwendungstext: Diese Diagnose enthält keinen Ersetzungsversuch zur Überprüfung.",
            Id => "Teks aplikasi: diagnosis ini tidak memiliki percobaan penggantian untuk diverifikasi.",
        }
    }

    /// Content-free evidence for recent route activity. Matcher previews are
    /// intentionally excluded because they do not exercise application output.
    pub(crate) fn why_pattern_status(
        &self,
        ibus_route: bool,
        ibus_latest_match: Option<wayexpand_backend_ibus::telemetry::IbusLatestMatch>,
        recent_outcome: Option<wayexpand_core::OutcomeKind>,
    ) -> &'static str {
        use wayexpand_backend_ibus::telemetry::IbusLatestMatch as M;
        use Language::{English as En, German as De, Indonesian as Id};
        if ibus_route && ibus_latest_match == Some(M::NoMatch) {
            return match self.lang {
                En => "Trigger match: no trigger matched the latest input in this field.",
                De => "Trigger: im zuletzt geprüften Feld passte kein Trigger zur Eingabe.",
                Id => "Pemicu: tidak ada pemicu yang cocok dengan masukan terakhir di kolom ini.",
            };
        }
        if ibus_route && matches!(ibus_latest_match, Some(M::OutputSubmitted | M::Refused)) {
            return match self.lang {
                En => "Trigger match: WayExpand recognized a trigger in the focused IBus field.",
                De => "Trigger: WayExpand hat im fokussierten IBus-Feld einen Trigger erkannt.",
                Id => "Pemicu: WayExpand mengenali pemicu di kolom IBus yang aktif.",
            };
        }
        let matched = !ibus_route && recent_outcome.is_some();
        match (matched, self.lang) {
            (true, En) => "Trigger match: confirmed for a recent output transaction.",
            (true, De) => "Trigger: für einen kürzlichen Ausgabevorgang erkannt.",
            (true, Id) => "Pemicu: terkonfirmasi pada transaksi keluaran terbaru.",
            (false, En) => "Trigger match: not confirmed by recent typing evidence.",
            (false, De) => "Trigger: durch aktuelle Eingaben nicht bestätigt.",
            (false, Id) => "Pemicu: belum terkonfirmasi dari aktivitas mengetik terbaru.",
        }
    }

    pub(crate) fn why_delivery_status(
        &self,
        ibus_route: bool,
        ibus_latest_match: Option<wayexpand_backend_ibus::telemetry::IbusLatestMatch>,
        recent_outcome: Option<wayexpand_core::OutcomeKind>,
        application_delivery_verified: bool,
    ) -> &'static str {
        use wayexpand_backend_ibus::telemetry::IbusLatestMatch as M;
        use wayexpand_core::OutcomeKind as O;
        use Language::{English as En, German as De, Indonesian as Id};
        if ibus_route {
            match ibus_latest_match {
                Some(M::OutputSubmitted) => {
                    return match self.lang {
                        En => {
                            "Application text: output was submitted, but the result is unverified."
                        }
                        De => "Anwendungstext: Ausgabe gesendet, Ergebnis aber nicht bestätigt.",
                        Id => {
                            "Teks aplikasi: keluaran dikirim, tetapi hasilnya belum terverifikasi."
                        }
                    };
                }
                Some(M::Refused) => {
                    return match self.lang {
                        En => "Application text: replacement was refused before output.",
                        De => "Anwendungstext: Ersetzung vor der Ausgabe abgelehnt.",
                        Id => "Teks aplikasi: penggantian ditolak sebelum keluaran.",
                    };
                }
                Some(M::NoMatch) => {
                    return match self.lang {
                        En => "Application text: no output was attempted for the latest input.",
                        De => {
                            "Anwendungstext: für die letzte Eingabe wurde keine Ausgabe versucht."
                        }
                        Id => "Teks aplikasi: tidak ada keluaran untuk masukan terakhir.",
                    };
                }
                Some(M::Unknown) | None => {}
            }
        }
        let recent_outcome = if ibus_route { None } else { recent_outcome };
        if application_delivery_verified && recent_outcome.is_none() {
            return match self.lang {
                En => "This route: verified only in the application used for the real test; other apps are not verified.",
                De => "Dieser Eingabeweg: in dieser Sitzung nur in der getesteten Anwendung bestätigt; andere Apps sind nicht verifiziert.",
                Id => "Rute ini: sesi ini hanya terverifikasi di aplikasi yang diuji; aplikasi lain belum terverifikasi.",
            };
        }
        match (recent_outcome, self.lang) {
            (Some(O::Expanded | O::ExpandedCursorMisplaced), En) => {
                "Application text: output was submitted, but the result is unverified."
            }
            (Some(O::Expanded | O::ExpandedCursorMisplaced), De) => {
                "Anwendungstext: Ausgabe gesendet, Ergebnis aber nicht bestätigt."
            }
            (Some(O::Expanded | O::ExpandedCursorMisplaced), Id) => {
                "Teks aplikasi: keluaran dikirim, tetapi hasilnya belum terverifikasi."
            }
            (Some(O::Unknown | O::Other), En) => {
                "Application text: delivery is uncertain; inspect it and do not retry."
            }
            (Some(O::Unknown | O::Other), De) => {
                "Anwendungstext: Ausgabe ungewiss; prüfen und nicht wiederholen."
            }
            (Some(O::Unknown | O::Other), Id) => {
                "Teks aplikasi: pengiriman tidak pasti; periksa dan jangan ulangi."
            }
            (Some(O::Partial), En) => {
                "Application text: possibly partial; inspect it and do not retry."
            }
            (Some(O::Partial), De) => {
                "Anwendungstext: möglicherweise unvollständig; prüfen und nicht wiederholen."
            }
            (Some(O::Partial), Id) => {
                "Teks aplikasi: mungkin tidak lengkap; periksa dan jangan ulangi."
            }
            (
                Some(
                    O::NotApplied
                    | O::BlockedByPolicy
                    | O::InterruptedByTyping
                    | O::SnippetActionFailed,
                ),
                En,
            ) => "Application text: replacement was not delivered.",
            (
                Some(
                    O::NotApplied
                    | O::BlockedByPolicy
                    | O::InterruptedByTyping
                    | O::SnippetActionFailed,
                ),
                De,
            ) => "Anwendungstext: Ersetzung wurde nicht gesendet.",
            (
                Some(
                    O::NotApplied
                    | O::BlockedByPolicy
                    | O::InterruptedByTyping
                    | O::SnippetActionFailed,
                ),
                Id,
            ) => "Teks aplikasi: penggantian tidak dikirim.",
            (None, En) => "Application text: no recent delivery is verified.",
            (None, De) => "Anwendungstext: keine aktuelle Ausgabe ist bestätigt.",
            (None, Id) => "Teks aplikasi: tidak ada keluaran terbaru yang terverifikasi.",
        }
    }

    pub fn problem_text(&self, problem: Problem) -> &'static str {
        use Language::{English as En, German as De, Indonesian as Id};
        match (problem, self.lang) {
            (Problem::InstallationIncomplete, En) => {
                "Some installed WayExpand components are missing, so setup may not survive an update or restart."
            }
            (Problem::InstallationIncomplete, De) => {
                "Einige installierte WayExpand-Komponenten fehlen. Die Einrichtung übersteht möglicherweise kein Update oder keinen Neustart."
            }
            (Problem::InstallationIncomplete, Id) => {
                "Beberapa komponen WayExpand yang terpasang hilang, jadi penyiapan mungkin tidak bertahan setelah pembaruan atau mulai ulang."
            }
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
            (Problem::NoEnabledExpansions, En) => {
                "WayExpand is receiving input, but the loaded configuration has no enabled keyboard expansions."
            }
            (Problem::NoEnabledExpansions, De) => {
                "WayExpand empfängt Eingaben, aber die geladene Konfiguration enthält keine aktivierten Tastatur-Erweiterungen."
            }
            (Problem::NoEnabledExpansions, Id) => {
                "WayExpand menerima input, tetapi konfigurasi yang dimuat tidak memiliki ekspansi keyboard yang aktif."
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
                "WayExpand cannot verify the running service's version or status format."
            }
            (Problem::BuildUnknown, De) => {
                "WayExpand kann die Version oder das Statusformat des laufenden Dienstes nicht verifizieren."
            }
            (Problem::BuildUnknown, Id) => {
                "WayExpand tidak dapat memverifikasi versi atau format status layanan yang berjalan."
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
            (Problem::GnomeSourceStatusUnavailable, En) => {
                "WayExpand cannot verify GNOME's active input source; the source-status check is unavailable."
            }
            (Problem::GnomeSourceStatusUnavailable, De) => {
                "WayExpand kann GNOMEs aktive Eingabequelle nicht überprüfen; die Quellenstatusprüfung ist nicht verfügbar."
            }
            (Problem::GnomeSourceStatusUnavailable, Id) => {
                "WayExpand tidak dapat memverifikasi sumber input GNOME yang aktif; pemeriksaan status sumber tidak tersedia."
            }
            (Problem::IbusEngineNotRegistered, En) => {
                "IBus cannot find WayExpand's engine, so it cannot start the input method."
            }
            (Problem::IbusEngineNotRegistered, De) => {
                "IBus findet die WayExpand-Engine nicht und kann die Eingabemethode daher nicht starten."
            }
            (Problem::IbusEngineNotRegistered, Id) => {
                "IBus tidak menemukan mesin WayExpand, jadi metode input tidak dapat dimulai."
            }
            (Problem::IbusManifestScopeInvalid, En) => {
                "The shared IBus registration points to a user-owned executable and is not safe for multi-user use."
            }
            (Problem::IbusManifestScopeInvalid, De) => {
                "Die gemeinsame IBus-Registrierung verweist auf ein benutzereigenes Programm und ist nicht für mehrere Benutzer sicher."
            }
            (Problem::IbusManifestScopeInvalid, Id) => {
                "Pendaftaran IBus bersama menunjuk ke executable milik pengguna dan tidak aman untuk banyak pengguna."
            }
            (Problem::IbusDesktopRouteUnavailable, En) => {
                "KWin is not routing keyboard input through IBus, so WayExpand cannot receive typing on this route."
            }
            (Problem::IbusDesktopRouteUnavailable, De) => {
                "KWin leitet Tastatureingaben nicht über IBus weiter, daher kann WayExpand sie über diesen Weg nicht empfangen."
            }
            (Problem::IbusDesktopRouteUnavailable, Id) => {
                "KWin tidak mengarahkan input keyboard melalui IBus, jadi WayExpand tidak dapat menerima ketikan melalui rute ini."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, En) => {
                "KWin's IBus Wayland helper has crashed recently, although it is running now. Input delivery needs a live test."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, De) => {
                "KWins IBus-Wayland-Hilfsprogramm ist kürzlich abgestürzt, läuft jetzt aber wieder. Die Eingabe muss live getestet werden."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, Id) => {
                "Helper IBus Wayland milik KWin baru-baru ini mengalami crash, meski sekarang berjalan. Pengiriman input perlu diuji langsung."
            }
            (Problem::InputSourceNotSelected, En) => {
                "Text expansion is unavailable because WayExpand is not the selected input source."
            }
            (Problem::InputSourceNotSelected, De) => {
                "Texterweiterungen sind nicht verfügbar, weil WayExpand nicht als Eingabequelle ausgewählt ist."
            }
            (Problem::InputSourceNotSelected, Id) => {
                "Ekspansi teks tidak tersedia karena WayExpand bukan sumber input yang dipilih."
            }
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
            (Problem::IbusInputNotObserved, En) => {
                "WayExpand is selected, but no focused IBus field has sent it typing to evaluate yet."
            }
            (Problem::IbusInputNotObserved, De) => {
                "WayExpand ist ausgewählt, aber noch kein fokussiertes IBus-Feld hat Eingaben zur Prüfung gesendet."
            }
            (Problem::IbusInputNotObserved, Id) => {
                "WayExpand dipilih, tetapi belum ada kolom IBus aktif yang mengirim ketikan untuk diperiksa."
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
                "In the most recently checked field, the application did not share enough nearby text for WayExpand to verify what a replacement would change. Automatic replacement is unavailable in that field. Use Quick Picker to copy a snippet, then paste it yourself."
            }
            (Problem::FieldCannotReplace, De) => {
                "Im zuletzt geprüften Feld hat die Anwendung WayExpand nicht genug Umgebungstext bereitgestellt, um die Änderung einer Ersetzung zu prüfen. Die automatische Ersetzung ist in diesem Feld nicht verfügbar. Kopieren Sie ein Snippet mit dem Schnellwähler und fügen Sie es selbst ein."
            }
            (Problem::FieldCannotReplace, Id) => {
                "Pada kolom yang terakhir diperiksa, aplikasi tidak memberikan cukup teks sekitar agar WayExpand dapat memeriksa perubahan penggantian. Penggantian otomatis tidak tersedia di kolom itu. Salin cuplikan dengan Quick Picker, lalu tempel sendiri."
            }
            (Problem::FieldTextMismatch, En) => {
                "WayExpand recognized a trigger, but the focused text or cursor did not match it, so replacement was refused."
            }
            (Problem::FieldTextMismatch, De) => {
                "WayExpand hat einen Trigger erkannt, aber Text oder Cursorposition passten nicht dazu. Die Ersetzung wurde abgelehnt."
            }
            (Problem::FieldTextMismatch, Id) => {
                "WayExpand mengenali pemicu, tetapi teks atau posisi kursor tidak cocok, jadi penggantian ditolak."
            }
            (Problem::FieldPolicyRefusal, En) => {
                "A policy or replacement limit refused this field's output; WayExpand did not submit it."
            }
            (Problem::FieldPolicyRefusal, De) => {
                "Eine Richtlinie oder Ersetzungsgrenze hat die Ausgabe für dieses Feld abgelehnt; WayExpand hat sie nicht gesendet."
            }
            (Problem::FieldPolicyRefusal, Id) => {
                "Kebijakan atau batas penggantian menolak keluaran untuk kolom ini; WayExpand tidak mengirimkannya."
            }
            (Problem::IbusFieldTypeUnknown, En) => {
                "IBus did not identify this as a normal text field, so WayExpand left capture off."
            }
            (Problem::IbusFieldTypeUnknown, De) => {
                "IBus hat dieses Feld nicht als normales Textfeld erkannt, deshalb blieb die Erfassung durch WayExpand ausgeschaltet."
            }
            (Problem::IbusFieldTypeUnknown, Id) => {
                "IBus tidak mengenali ini sebagai kolom teks biasa, jadi WayExpand tidak mengaktifkan penangkapan input."
            }
            (Problem::SensitiveField, En) => {
                "The last field was marked sensitive, so WayExpand intentionally did not capture or expand its typing."
            }
            (Problem::SensitiveField, De) => {
                "Das letzte Feld wurde als sensibel markiert. WayExpand hat dort absichtlich keine Eingaben erfasst oder erweitert."
            }
            (Problem::SensitiveField, Id) => {
                "Kolom terakhir ditandai sensitif, jadi WayExpand sengaja tidak menangkap atau memperluas ketikan di sana."
            }
            (Problem::TriggerNotMatched, En) => {
                "WayExpand received input, but no trigger matched the latest input in this field."
            }
            (Problem::TriggerNotMatched, De) => {
                "WayExpand hat Eingaben empfangen, aber im zuletzt geprüften Feld passte kein Trigger zur Eingabe."
            }
            (Problem::TriggerNotMatched, Id) => {
                "WayExpand menerima masukan, tetapi tidak ada pemicu yang cocok dengan masukan terakhir di kolom ini."
            }
            (Problem::TriggerMatchUnknown, En) => {
                "A real insertion was confirmed only in the application named by the earlier test. Recent activity in this field does not show whether your intended trigger matched. Check the snippet here, then test delivery in this application; a matcher preview cannot verify it."
            }
            (Problem::TriggerMatchUnknown, De) => {
                "Eine echte Einfügung wurde nur in der Anwendung bestätigt, die beim vorherigen Test genannt wurde. Die letzten Eingaben in diesem Feld zeigen nicht, ob Ihr gewünschter Trigger erkannt wurde. Prüfen Sie das Snippet und testen Sie die Ausgabe in dieser Anwendung; eine Matcher-Vorschau kann das nicht bestätigen."
            }
            (Problem::TriggerMatchUnknown, Id) => {
                "Penyisipan nyata hanya dikonfirmasi pada aplikasi yang disebut dalam tes sebelumnya. Aktivitas terbaru di kolom ini tidak menunjukkan apakah pemicu yang Anda maksud cocok. Periksa cuplikan lalu uji pengiriman di aplikasi ini; pratinjau matcher tidak dapat memverifikasinya."
            }
            (Problem::TerminalFocusUnknown, En) => {
                "The terminal-only route is failing closed because WayExpand cannot identify the currently focused application."
            }
            (Problem::TerminalFocusUnknown, De) => {
                "Der nur für Terminals konfigurierte Eingabeweg bleibt gesperrt, weil WayExpand die aktuell fokussierte Anwendung nicht erkennen kann."
            }
            (Problem::TerminalFocusUnknown, Id) => {
                "Rute khusus terminal ditutup dengan aman karena WayExpand tidak dapat mengenali aplikasi yang sedang aktif."
            }
            (Problem::TerminalCompositorIdentity, En) => {
                "KWin's focus tracker is connected, but it reported KWin's own process ID instead of the focused terminal's application ID. The terminal-only route is blocking matching."
            }
            (Problem::TerminalCompositorIdentity, De) => {
                "KWin-Fokus-Tracking ist verbunden, meldet aber die Prozess-ID von KWin statt der Anwendung des fokussierten Terminals. Der nur für Terminals konfigurierte Eingabeweg blockiert den Abgleich."
            }
            (Problem::TerminalCompositorIdentity, Id) => {
                "Pelacak fokus KWin terhubung, tetapi melaporkan ID proses KWin, bukan ID aplikasi terminal yang aktif. Rute khusus terminal memblokir pencocokan."
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
            (Problem::TerminalRouteNoInput, En) => {
                "No typing has reached WayExpand since this service started. It only handles your terminal apps."
            }
            (Problem::TerminalRouteNoInput, De) => {
                "Seit dem Start dieses Dienstes hat WayExpand keine Eingabe erhalten. Diese Route funktioniert nur in Ihren Terminal-Apps."
            }
            (Problem::TerminalRouteNoInput, Id) => {
                "Sejak layanan ini dimulai, WayExpand belum menerima ketikan. Rute ini hanya berfungsi di aplikasi terminal Anda."
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
            (Action::CheckInstallation, En) => "Check installation",
            (Action::CheckInstallation, De) => "Installation prüfen",
            (Action::CheckInstallation, Id) => "Periksa instalasi",
            (Action::TurnOn, En) => "Turn on",
            (Action::TurnOn, De) => "Einschalten",
            (Action::TurnOn, Id) => "Aktifkan",
            (Action::Resume, En) => "Resume",
            (Action::Resume, De) => "Fortsetzen",
            (Action::Resume, Id) => "Lanjutkan",
            (Action::RunSetup, En) => "Fix with guided setup",
            (Action::RunSetup, De) => "Mit geführter Einrichtung beheben",
            (Action::RunSetup, Id) => "Perbaiki dengan penyiapan terpandu",
            (Action::CheckUpgrade, En) => "Check versions and restart safely",
            (Action::CheckUpgrade, De) => "Versionen prüfen und sicher neu starten",
            (Action::CheckUpgrade, Id) => "Periksa versi dan mulai ulang dengan aman",
            (Action::SelectInputSource, En) => "Select WayExpand",
            (Action::SelectInputSource, De) => "WayExpand auswählen",
            (Action::SelectInputSource, Id) => "Pilih WayExpand",
            (Action::ReviewConsent, En) => "Review and allow",
            (Action::ReviewConsent, De) => "Prüfen und erlauben",
            (Action::ReviewConsent, Id) => "Tinjau dan izinkan",
            (Action::CheckAgain, En) => "Check again",
            (Action::CheckAgain, De) => "Erneut prüfen",
            (Action::CheckAgain, Id) => "Periksa lagi",
            (Action::QuickPicker, En) => "Copy a snippet…",
            (Action::QuickPicker, De) => "Snippet kopieren…",
            (Action::QuickPicker, Id) => "Salin cuplikan…",
            (Action::OpenMatcherPreview, En) => "Open Matcher Preview",
            (Action::OpenMatcherPreview, De) => "Matcher-Vorschau öffnen",
            (Action::OpenMatcherPreview, Id) => "Buka Pratinjau Matcher",
            (Action::TestInApplication, En) => "Test in an app",
            (Action::TestInApplication, De) => "In einer App testen",
            (Action::TestInApplication, Id) => "Uji di aplikasi",
            (Action::InspectText, En) => "Refresh diagnostics",
            (Action::InspectText, De) => "Diagnose aktualisieren",
            (Action::InspectText, Id) => "Perbarui diagnostik",
            (Action::ReviewSnippet, En) => "Back to snippets",
            (Action::ReviewSnippet, De) => "Zurück zu den Snippets",
            (Action::ReviewSnippet, Id) => "Kembali ke cuplikan",
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
            (Problem::InstallationIncomplete, En) => {
                "Run the read-only installation and version check. It lists the missing components and offers only repairs that match the active installation."
            }
            (Problem::InstallationIncomplete, De) => {
                "Führen Sie die schreibgeschützte Installations- und Versionsprüfung aus. Sie listet fehlende Komponenten auf und bietet nur Reparaturen für die aktive Installation an."
            }
            (Problem::InstallationIncomplete, Id) => {
                "Jalankan pemeriksaan instalasi dan versi yang hanya membaca. Laporan menunjukkan komponen yang hilang dan hanya menawarkan perbaikan untuk instalasi aktif."
            }
            (Problem::RestartNeeded, En) => {
                "Run the read-only version check first. It verifies which installation owns the daemon and only offers a restart when it identifies the matching user service."
            }
            (Problem::RestartNeeded, De) => {
                "Führen Sie zuerst die schreibgeschützte Versionsprüfung aus. Sie ermittelt die zuständige Installation und bietet einen Neustart nur für den passenden Benutzerdienst an."
            }
            (Problem::RestartNeeded, Id) => {
                "Jalankan pemeriksaan versi hanya-baca terlebih dahulu. Pemeriksaan ini memastikan instalasi yang memiliki daemon dan hanya menawarkan mulai ulang jika layanan pengguna yang tepat ditemukan."
            }
            (Problem::BuildUnknown, En) => {
                "The daemon build cannot be verified. Run the read-only version check and follow its installation-specific next step; do not restart an unidentified service."
            }
            (Problem::BuildUnknown, De) => {
                "Der Daemon-Build kann nicht verifiziert werden. Führen Sie die schreibgeschützte Versionsprüfung aus und folgen Sie dem installationsspezifischen Hinweis. Starten Sie keinen nicht zugeordneten Dienst neu."
            }
            (Problem::BuildUnknown, Id) => {
                "Build daemon tidak dapat diverifikasi. Jalankan pemeriksaan versi hanya-baca dan ikuti langkah sesuai instalasi; jangan mulai ulang layanan yang tidak teridentifikasi."
            }
            (Problem::Paused, En) => {
                "Select Resume in the menu at the top of the window, then retry your trigger. Quick Picker remains available for copy and paste."
            }
            (Problem::Paused, De) => {
                "Wählen Sie oben im Fenstermenü „Fortsetzen“ und versuchen Sie den Trigger erneut. Der Schnellwähler bleibt zum Kopieren und Einfügen verfügbar."
            }
            (Problem::Paused, Id) => {
                "Pilih Lanjutkan di menu bagian atas jendela, lalu coba pemicu lagi. Quick Picker tetap tersedia untuk menyalin dan menempel."
            }
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
            (Problem::IbusInputNotObserved, En) => {
                "Run the real-application test below in a normal text field. It reports input capture separately from trigger matching and visible replacement, so you can see which stage failed."
            }
            (Problem::IbusInputNotObserved, De) => {
                "Führen Sie unten den Anwendungstest in einem normalen Textfeld aus. Er zeigt Eingabeerfassung, Trigger-Erkennung und sichtbare Ersetzung getrennt, damit die fehlerhafte Stufe erkennbar ist."
            }
            (Problem::IbusInputNotObserved, Id) => {
                "Jalankan tes aplikasi nyata di bawah dalam kolom teks biasa. Hasilnya memisahkan penangkapan input, pencocokan pemicu, dan penggantian yang terlihat agar tahap yang gagal jelas."
            }
            (Problem::TriggerNotMatched, En) => {
                "Telemetry does not record which trigger you intended or this field's app identity. If that input was part of your trigger, select the snippet and check its exact trigger, enabled state, match mode, and app filters in Matcher Preview. A preview match still needs a real-app typing test to verify delivery."
            }
            (Problem::TriggerNotMatched, De) => {
                "Die Diagnose speichert weder Ihren beabsichtigten Trigger noch die App-Identität dieses Felds. Falls diese Eingabe Teil Ihres Triggers war, prüfen Sie im Snippet den exakten Trigger, Aktivierungsstatus, Abgleichmodus und App-Filter. Ein Vorschau-Treffer muss anschließend in einer echten Anwendung getestet werden."
            }
            (Problem::TriggerNotMatched, Id) => {
                "Telemetri tidak menyimpan pemicu yang Anda maksud atau identitas aplikasi kolom ini. Jika masukan itu bagian dari pemicu, periksa pemicu persis, status aktif, mode pencocokan, dan filter aplikasi di cuplikan. Kecocokan pratinjau tetap perlu diuji di aplikasi nyata untuk memastikan teks terkirim."
            }
            (Problem::NoEnabledExpansions, En) => {
                "Open Snippets, enable a keyboard expansion or create a text expansion, and save. WayExpand reloads after saving; follow any reload warning, then type its trigger in the affected app and verify the visible replacement."
            }
            (Problem::NoEnabledExpansions, De) => {
                "Öffnen Sie Snippets, aktivieren oder erstellen Sie eine Tastatur-Erweiterung und speichern Sie. WayExpand lädt danach neu. Folgen Sie einem angezeigten Ladefehler, geben Sie anschließend den Trigger in der betroffenen App ein und prüfen Sie den sichtbaren Ersatz."
            }
            (Problem::NoEnabledExpansions, Id) => {
                "Buka Cuplikan, aktifkan ekspansi keyboard atau buat ekspansi teks, lalu simpan. WayExpand akan memuat ulang setelah disimpan; ikuti peringatan jika pemuatan gagal, lalu ketik pemicunya di aplikasi terkait dan pastikan teks penggantinya terlihat."
            }
            (Problem::TriggerMatchUnknown, En) => {
                "Select the intended snippet and check its exact trigger, enabled state, match mode, and app filters in Matcher Preview. If it matches, type that trigger in this app; the preview does not prove text delivery."
            }
            (Problem::TriggerMatchUnknown, De) => {
                "Wählen Sie das gewünschte Snippet und prüfen Sie Trigger, Aktivierung, Abgleichmodus und App-Filter in der Matcher-Vorschau. Geben Sie den Trigger anschließend in dieser App ein; die Vorschau bestätigt keine Textübertragung."
            }
            (Problem::TriggerMatchUnknown, Id) => {
                "Pilih cuplikan yang dimaksud dan periksa pemicu, status aktif, mode pencocokan, serta filter aplikasi di Pratinjau Matcher. Jika cocok, ketik pemicu itu di aplikasi ini; pratinjau tidak membuktikan teks terkirim."
            }
            (Problem::TerminalFocusUnknown, En) => {
                "Immediate workaround: open Quick Picker, copy the snippet, and paste it in the terminal. Automatic matching is blocked until WayExpand can identify the focused app. To diagnose that, focus the terminal and run `wayexpand debug --focused-app-id --json`; if `focused_app_id` is null, follow its problem ID and suggested action. Do not allowlist a compositor or window title. Retest once a verified terminal app ID is available."
            }
            (Problem::TerminalFocusUnknown, De) => {
                "Sofortige Umgehung: Öffnen Sie den Schnellwähler, kopieren Sie das Snippet und fügen Sie es im Terminal ein. Der automatische Abgleich bleibt gesperrt, bis WayExpand die fokussierte Anwendung erkennt. Zur Diagnose fokussieren Sie das Terminal und führen `wayexpand debug --focused-app-id --json` aus. Ist `focused_app_id` null, folgen Sie der Problem-ID und Handlungsempfehlung. Nehmen Sie weder einen Compositor noch einen Fenstertitel in die Zulassungsliste auf. Testen Sie erneut, sobald eine verifizierte Terminal-App-ID verfügbar ist."
            }
            (Problem::TerminalFocusUnknown, Id) => {
                "Solusi sementara: buka Quick Picker, salin cuplikan, lalu tempelkan di terminal. Pencocokan otomatis diblokir sampai WayExpand dapat mengenali aplikasi yang aktif. Untuk mendiagnosisnya, fokuskan terminal dan jalankan `wayexpand debug --focused-app-id --json`; jika `focused_app_id` bernilai null, ikuti ID masalah dan saran tindakannya. Jangan izinkan compositor atau judul jendela. Uji kembali setelah ID aplikasi terminal terverifikasi tersedia."
            }
            (Problem::TerminalCompositorIdentity, En) => {
                "KWin returned its own process ID, so WayExpand blocked matching before attempting replacement. There is no allowlist setting that fixes this identity result. Use Quick Picker in copy-only mode and paste in the terminal. Retest automatic matching only after KWin reports a verified client application ID; do not allowlist `kwin_wayland` or a window title."
            }
            (Problem::TerminalCompositorIdentity, De) => {
                "KWin hat seine eigene Prozess-ID zurückgegeben. Deshalb wurde der Abgleich vor einer Ersetzung blockiert. Keine Zulassungsliste kann dieses Identitätsergebnis beheben. Nutzen Sie den Schnellwähler im Nur-Kopieren-Modus und fügen Sie den Text im Terminal ein. Testen Sie den automatischen Abgleich erst erneut, wenn KWin eine verifizierte Client-Anwendungs-ID meldet. Nehmen Sie weder `kwin_wayland` noch einen Fenstertitel in die Zulassungsliste auf."
            }
            (Problem::TerminalCompositorIdentity, Id) => {
                "KWin mengembalikan ID prosesnya sendiri, jadi pencocokan diblokir sebelum penggantian dicoba. Tidak ada pengaturan daftar izin yang dapat memperbaiki hasil identitas ini. Gunakan Quick Picker dalam mode salin saja lalu tempel di terminal. Uji pencocokan otomatis kembali hanya setelah KWin melaporkan ID aplikasi klien yang terverifikasi; jangan izinkan `kwin_wayland` atau judul jendela."
            }
            (Problem::FieldTextMismatch, En) => {
                "Check the exact trigger in Matcher Preview, then place the cursor immediately after it with no selection or autocorrect change. Inspect the field before trying again; use Quick Picker to paste if the mismatch continues."
            }
            (Problem::FieldTextMismatch, De) => {
                "Prüfen Sie den exakten Trigger in der Matcher-Vorschau. Setzen Sie den Cursor direkt dahinter, ohne Auswahl oder Autokorrektur. Prüfen Sie das Feld vor einem neuen Versuch; bei weiterem Konflikt nutzen Sie den Schnellwähler."
            }
            (Problem::FieldTextMismatch, Id) => {
                "Periksa pemicu persisnya di Pratinjau Matcher, lalu tempatkan kursor tepat setelah pemicu tanpa seleksi atau perubahan koreksi otomatis. Periksa kolom sebelum mencoba lagi; gunakan Quick Picker jika tetap tidak cocok."
            }
            (Problem::FieldPolicyRefusal, En) => {
                "Open Diagnostics → Policy to see the refusing rule or output limit. Ask the policy administrator before changing an enforced rule; use Quick Picker only if copying is allowed."
            }
            (Problem::FieldPolicyRefusal, De) => {
                "Öffnen Sie Diagnose → Richtlinie und prüfen Sie die ablehnende Regel oder Ausgabebegrenzung. Fragen Sie vor Änderungen an erzwungenen Regeln die Administration; nutzen Sie den Schnellwähler nur, wenn Kopieren erlaubt ist."
            }
            (Problem::FieldPolicyRefusal, Id) => {
                "Buka Diagnostik → Kebijakan untuk melihat aturan atau batas keluaran yang menolak. Tanyakan kepada administrator sebelum mengubah aturan wajib; gunakan Quick Picker hanya jika penyalinan diizinkan."
            }
            (Problem::IbusFieldTypeUnknown, En) => {
                "Run the real-application test in a normal text field. If capture remains unavailable, switch away from WayExpand and back, then retry; field-type checks stay enabled to protect sensitive inputs."
            }
            (Problem::IbusFieldTypeUnknown, De) => {
                "Führen Sie den Anwendungstest in einem normalen Textfeld aus. Falls die Erfassung weiter fehlt, wechseln Sie kurz von WayExpand weg und zurück. Die Feldtypprüfung bleibt zum Schutz sensibler Eingaben aktiv."
            }
            (Problem::IbusFieldTypeUnknown, Id) => {
                "Jalankan tes aplikasi nyata di kolom teks biasa. Jika input tetap tidak tertangkap, alihkan metode input dari WayExpand lalu pilih kembali. Pemeriksaan jenis kolom tetap aktif untuk melindungi input sensitif."
            }
            (Problem::InputSourceNotSelected, En) if gnome_ibus => {
                "Text expansion is unavailable because GNOME has not selected WayExpand. Use Select WayExpand below; if it remains unavailable, choose WayExpand in Settings → Keyboard or the input-source menu. Then run a live typing test in the affected app. Quick Picker can copy a snippet for you to paste meanwhile."
            }
            (Problem::InputSourceNotSelected, De) if gnome_ibus => {
                "Texterweiterungen sind nicht verfügbar, weil GNOME WayExpand nicht ausgewählt hat. Nutzen Sie unten „WayExpand auswählen“. Falls WayExpand danach nicht aktiv ist, wählen Sie es unter Einstellungen → Tastatur oder im Eingabequellen-Menü aus. Führen Sie anschließend einen Live-Tipptest in der betroffenen App durch. Bis dahin können Sie ein Snippet mit dem Schnellwähler kopieren und einfügen."
            }
            (Problem::InputSourceNotSelected, Id) if gnome_ibus => {
                "Ekspansi teks tidak tersedia karena GNOME belum memilih WayExpand. Gunakan tombol Pilih WayExpand di bawah; jika WayExpand tetap belum aktif, pilih di Pengaturan → Keyboard atau menu sumber input. Lalu jalankan tes mengetik langsung di aplikasi yang bermasalah. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel."
            }
            (Problem::GnomeSourceStatusUnavailable, En) => {
                "WayExpand could not verify GNOME's active input source. Open Diagnostics → IBus to inspect the source-status error, then run `wayexpand doctor --gnome`. Use guided setup only if those checks identify a missing component. Quick Picker can copy a snippet for manual paste meanwhile."
            }
            (Problem::GnomeSourceStatusUnavailable, De) => {
                "WayExpand konnte GNOMEs aktive Eingabequelle nicht überprüfen. Öffnen Sie Diagnose → IBus, prüfen Sie den Fehler zum Quellenstatus und führen Sie `wayexpand doctor --gnome` aus. Nutzen Sie die geführte Einrichtung nur, wenn diese Prüfungen eine fehlende Komponente feststellen. Bis dahin kann der Schnellwähler ein Snippet kopieren."
            }
            (Problem::GnomeSourceStatusUnavailable, Id) => {
                "WayExpand tidak dapat memverifikasi sumber input GNOME yang aktif. Buka Diagnostik → IBus untuk memeriksa kesalahan status sumber, lalu jalankan `wayexpand doctor --gnome`. Gunakan penyiapan terpandu hanya jika pemeriksaan menunjukkan komponen yang hilang. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel manual."
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
            (Problem::IbusManifestScopeInvalid, En) => {
                "Run guided IBus setup to replace the shared registration with a system-owned engine, or install WayExpand from a distribution package. Setup may request administrator authorization. Then refresh Diagnostics → IBus and run a real-app typing test."
            }
            (Problem::IbusManifestScopeInvalid, De) => {
                "Starten Sie die geführte IBus-Einrichtung, um die gemeinsame Registrierung durch eine systemeigene Engine zu ersetzen, oder installieren Sie WayExpand als Distributionspaket. Möglicherweise ist eine Administratorfreigabe nötig. Aktualisieren Sie danach Diagnose → IBus und testen Sie die Eingabe in einer echten Anwendung."
            }
            (Problem::IbusManifestScopeInvalid, Id) => {
                "Jalankan penyiapan IBus terpandu untuk mengganti pendaftaran bersama dengan mesin milik sistem, atau pasang WayExpand dari paket distribusi. Penyiapan mungkin meminta izin administrator. Lalu perbarui Diagnostik → IBus dan uji pengetikan di aplikasi nyata."
            }
            (Problem::IbusDesktopRouteUnavailable, En) => {
                "Use guided setup to configure KWin's IBus Wayland route, then run a harmless live typing test. If setup cannot apply the change, select IBus Wayland in KDE System Settings → Keyboard → Virtual Keyboard and check again. Quick Picker can copy a snippet meanwhile."
            }
            (Problem::IbusDesktopRouteUnavailable, De) => {
                "Nutzen Sie die geführte Einrichtung, um KWin für IBus Wayland zu konfigurieren, und testen Sie danach harmlose Eingaben live. Falls die Einrichtung die Änderung nicht anwenden kann, wählen Sie „IBus Wayland“ in den KDE-Systemeinstellungen unter Tastatur → Virtuelle Tastatur und prüfen Sie erneut. Bis dahin kann der Schnellwähler ein Snippet kopieren."
            }
            (Problem::IbusDesktopRouteUnavailable, Id) => {
                "Gunakan penyiapan terpandu untuk mengatur rute IBus Wayland KWin, lalu jalankan tes mengetik langsung yang aman. Jika penyiapan tidak dapat menerapkan perubahan, pilih IBus Wayland di Pengaturan Sistem KDE → Keyboard → Virtual Keyboard, lalu periksa lagi. Sementara itu, Quick Picker dapat menyalin cuplikan."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, En) => {
                "Run the real-app test in the affected field. If it receives no input, run `wayexpand ibus watch --seconds 30`, then `coredumpctl info ibus-ui-gtk3` and check for an IBus package update. Quick Picker can copy a snippet meanwhile."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, De) => {
                "Führen Sie den Anwendungstest im betroffenen Feld aus. Wenn keine Eingabe ankommt, starten Sie `wayexpand ibus watch --seconds 30`, dann `coredumpctl info ibus-ui-gtk3`, und suchen Sie nach einer IBus-Paketaktualisierung. Bis dahin kann der Schnellwähler ein Snippet kopieren."
            }
            (Problem::IbusDesktopRouteRecentlyCrashed, Id) => {
                "Jalankan tes aplikasi nyata pada kolom yang bermasalah. Jika input tidak diterima, jalankan `wayexpand ibus watch --seconds 30`, lalu `coredumpctl info ibus-ui-gtk3` dan periksa pembaruan paket IBus. Sementara itu, Quick Picker dapat menyalin cuplikan."
            }
            (Problem::IbusEngineUnavailable, En) => {
                "Use guided setup below to reconnect the selected IBus engine, then run a live typing test. If setup asks for a GNOME logout/login, complete that first. Quick Picker can copy a snippet for manual paste meanwhile."
            }
            (Problem::IbusEngineUnavailable, De) => {
                "Nutzen Sie unten die geführte Einrichtung, um die ausgewählte IBus-Engine erneut zu verbinden, und führen Sie danach einen Live-Tipptest durch. Falls die Einrichtung eine Ab- und Anmeldung bei GNOME verlangt, erledigen Sie dies zuerst. Bis dahin kann der Schnellwähler ein Snippet zum manuellen Einfügen kopieren."
            }
            (Problem::IbusEngineUnavailable, Id) => {
                "Gunakan penyiapan terpandu di bawah untuk menyambungkan kembali mesin IBus yang dipilih, lalu lakukan tes mengetik langsung. Jika penyiapan meminta logout/login GNOME, lakukan itu terlebih dahulu. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel manual."
            }
            (Problem::InputSourceNotSelected, En) => {
                "Run guided IBus setup to select WayExpand, then run a live typing test. Quick Picker can copy a snippet for you to paste meanwhile."
            }
            (Problem::InputSourceNotSelected, De) => {
                "Führen Sie die geführte IBus-Einrichtung aus, um WayExpand auszuwählen, und starten Sie danach einen Live-Tipptest. Bis dahin können Sie ein Snippet mit dem Schnellwähler kopieren und einfügen."
            }
            (Problem::InputSourceNotSelected, Id) => {
                "Jalankan penyiapan IBus terpandu untuk memilih WayExpand, lalu lakukan tes mengetik langsung. Sementara itu, Quick Picker dapat menyalin cuplikan untuk ditempel."
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
            (Problem::TerminalRouteNoInput, En) => {
                "Type the test trigger in a configured terminal and confirm the replacement. Browser and editor fields are outside this route; use Quick Picker to copy and paste there."
            }
            (Problem::TerminalRouteNoInput, De) => {
                "Geben Sie den Test-Trigger in einem konfigurierten Terminal ein und prüfen Sie den Ersatz. Browser- und Editorfelder liegen außerhalb dieser Route; nutzen Sie dort den Schnellwähler zum Kopieren und Einfügen."
            }
            (Problem::TerminalRouteNoInput, Id) => {
                "Ketik pemicu uji di terminal yang dikonfigurasi dan pastikan teks pengganti muncul. Kolom browser dan editor tidak tercakup; gunakan Quick Picker untuk menyalin dan menempel di sana."
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
            (Problem::SnippetActionFailed, En) => "Return to the snippet library and select the item you just used to review its form fields or command result. WayExpand does not rerun a failed command automatically.",
            (Problem::SnippetActionFailed, De) => "Kehren Sie zur Snippet-Bibliothek zurück und wählen Sie das gerade verwendete Element aus, um Formularfelder oder Befehlsausgabe zu prüfen. WayExpand führt einen fehlgeschlagenen Befehl nicht automatisch erneut aus.",
            (Problem::SnippetActionFailed, Id) => "Kembali ke pustaka cuplikan dan pilih item yang baru digunakan untuk memeriksa kolom formulir atau hasil perintah. WayExpand tidak menjalankan ulang perintah yang gagal secara otomatis.",
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
                "WayExpand received typing in the most recently checked field, but the application did not provide enough nearby text to verify replacement. No automatic replacement was submitted. Open Quick Picker in copy-only mode and paste the snippet there."
            }
            (Problem::FieldCannotReplace, De) => {
                "WayExpand hat Eingaben im zuletzt geprüften Feld erkannt, aber die Anwendung hat nicht genug Umgebungstext bereitgestellt, um die Ersetzung zu prüfen. Es wurde keine automatische Ersetzung gesendet. Öffnen Sie den Schnellwähler im Kopiermodus und fügen Sie das Snippet dort ein."
            }
            (Problem::FieldCannotReplace, Id) => {
                "WayExpand menerima pengetikan di kolom yang terakhir diperiksa, tetapi aplikasi tidak memberikan cukup teks sekitar untuk memeriksa penggantian. Tidak ada penggantian otomatis yang dikirim. Buka Quick Picker dalam mode salin saja dan tempel cuplikan di sana."
            }
            (Problem::SensitiveField, En) => {
                "Test only in a normal, non-sensitive text field. Choose Desktop Integration Test and confirm only if the exact replacement appears."
            }
            (Problem::SensitiveField, De) => {
                "Testen Sie nur in einem normalen, nicht sensiblen Textfeld. Wählen Sie den Desktop-Integrationstest und bestätigen Sie nur, wenn genau die Ersetzung erscheint."
            }
            (Problem::SensitiveField, Id) => {
                "Uji hanya di kolom teks biasa yang tidak sensitif. Pilih Uji Integrasi Desktop dan konfirmasi hanya jika teks pengganti yang tepat muncul."
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
            (Staleness::ConfigurationChanged, Language::English) => {
                "the snippet configuration changed since the test".into()
            }
            (Staleness::ConfigurationChanged, Language::German) => {
                "die Snippet-Konfiguration wurde seit dem Test geändert".into()
            }
            (Staleness::ConfigurationChanged, Language::Indonesian) => {
                "konfigurasi cuplikan berubah sejak pengujian".into()
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
    pub(crate) fn upgrade_cli_mismatch(&self, active_cli: Option<&str>) -> String {
        let path = active_cli.unwrap_or("the active installation CLI");
        match self.lang {
            Language::English => format!(
                "This GUI's CLI does not match the active installation. Run `{path} upgrade --check` and follow that report; no service was restarted."
            ),
            Language::German => format!(
                "Die CLI dieser GUI gehört nicht zur aktiven Installation. Führen Sie `{path} upgrade --check` aus und folgen Sie dem Bericht. Es wurde kein Dienst neu gestartet."
            ),
            Language::Indonesian => format!(
                "CLI GUI ini tidak cocok dengan instalasi aktif. Jalankan `{path} upgrade --check` dan ikuti laporannya; tidak ada layanan yang dimulai ulang."
            ),
        }
    }

    pub(crate) fn upgrade_restart_available(&self) -> &'static str {
        match self.lang {
            Language::English => "The version check found the matching WayExpand service. Review the restart prompt to continue.",
            Language::German => "Die Versionsprüfung hat den passenden WayExpand-Dienst gefunden. Prüfen Sie die Neustartabfrage, um fortzufahren.",
            Language::Indonesian => "Pemeriksaan versi menemukan layanan WayExpand yang sesuai. Tinjau konfirmasi mulai ulang untuk melanjutkan.",
        }
    }

    pub(crate) fn upgrade_restart_unmanaged(&self) -> &'static str {
        match self.lang {
            Language::English => "The daemon build differs, but no matching managed service was identified. Do not restart another service; inspect Diagnostics → Services.",
            Language::German => "Der Daemon-Build unterscheidet sich, aber es wurde kein passender verwalteter Dienst gefunden. Starten Sie keinen anderen Dienst neu; prüfen Sie Diagnose → Dienste.",
            Language::Indonesian => "Build daemon berbeda, tetapi layanan terkelola yang sesuai tidak ditemukan. Jangan mulai ulang layanan lain; periksa Diagnostik → Layanan.",
        }
    }

    pub(crate) fn upgrade_build_current(&self) -> &'static str {
        match self.lang {
            Language::English => "The running daemon matches this GUI build. Refresh diagnostics to check the input route.",
            Language::German => "Der laufende Daemon entspricht diesem GUI-Build. Aktualisieren Sie die Diagnose, um den Eingabeweg zu prüfen.",
            Language::Indonesian => "Daemon yang berjalan cocok dengan build GUI ini. Perbarui diagnostik untuk memeriksa rute input.",
        }
    }

    pub(crate) fn upgrade_check_unreadable(&self) -> &'static str {
        match self.lang {
            Language::English => "The version check did not return a readable report. Open Diagnostics → Versions and run `wayexpand upgrade --check --json`.",
            Language::German => "Die Versionsprüfung lieferte keinen lesbaren Bericht. Öffnen Sie Diagnose → Versionen und führen Sie `wayexpand upgrade --check --json` aus.",
            Language::Indonesian => "Pemeriksaan versi tidak menghasilkan laporan yang dapat dibaca. Buka Diagnostik → Versi dan jalankan `wayexpand upgrade --check --json`.",
        }
    }

    pub(crate) fn upgrade_installation_incomplete(&self) -> &'static str {
        match self.lang {
            Language::English => "The upgrade check found missing installed components. Open Diagnostics and run `wayexpand install-check --json` to see what needs repair.",
            Language::German => "Die Upgrade-Prüfung hat fehlende installierte Komponenten gefunden. Öffnen Sie Diagnose und führen Sie `wayexpand install-check --json` aus, um die nötige Reparatur zu sehen.",
            Language::Indonesian => "Pemeriksaan upgrade menemukan komponen terpasang yang hilang. Buka Diagnostik dan jalankan `wayexpand install-check --json` untuk melihat perbaikannya.",
        }
    }

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
