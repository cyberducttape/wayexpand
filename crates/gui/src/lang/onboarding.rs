//! First-run guide and guided-setup text.

use super::{Language, Strings};

/// One `&'static str` method per entry: English, German, Indonesian.
macro_rules! texts {
    ($($name:ident => $en:expr, $de:expr, $id:expr;)*) => {
        impl Strings {
            $(
                pub fn $name(&self) -> &'static str {
                    match self.lang {
                        Language::English => $en,
                        Language::German => $de,
                        Language::Indonesian => $id,
                    }
                }
            )*
        }
    };
}

texts! {
    outcome_working =>
        "Working in tested application",
        "Funktioniert in der getesteten Anwendung",
        "Berfungsi di aplikasi yang diuji";
    outcome_safe_fallback =>
        "Safe fallback",
        "Sichere Ausweichlösung",
        "Cadangan aman";
    outcome_setup_incomplete =>
        "Setup incomplete",
        "Einrichtung unvollständig",
        "Penyiapan belum selesai";
    verification_not_tested =>
        "Verified in tested application: not tested",
        "In getesteter Anwendung verifiziert: nicht getestet",
        "Terverifikasi di aplikasi yang diuji: belum diuji";
    verification_unnamed_application =>
        "Verified in tested application: the tested application",
        "In getesteter Anwendung verifiziert: die getestete Anwendung",
        "Terverifikasi di aplikasi yang diuji: aplikasi yang diuji";
    status_create_test_expansion_first =>
        "Create or enable the harmless WayExpand test expansion before testing insertion.",
        "Legen Sie die harmlose WayExpand-Testerweiterung an oder aktivieren Sie sie, bevor Sie das Einfügen testen.",
        "Buat atau aktifkan ekspansi uji WayExpand yang aman sebelum menguji penyisipan.";
    status_first_run_not_complete =>
        "First run is not complete: verify an exact insertion in a real application first.",
        "Die Ersteinrichtung ist nicht abgeschlossen: Prüfen Sie zuerst ein exaktes Einfügen in einer echten Anwendung.",
        "Penggunaan pertama belum selesai: verifikasi dulu penyisipan yang tepat di aplikasi nyata.";
    status_setup_deferred =>
        "Setup is not verified yet. The first-run guide will return until a real application test succeeds.",
        "Die Einrichtung ist noch nicht verifiziert. Der Einrichtungsassistent erscheint wieder, bis ein Test in einer echten Anwendung gelingt.",
        "Penyiapan belum terverifikasi. Panduan penggunaan pertama akan muncul lagi sampai uji di aplikasi nyata berhasil.";
    status_picker_copy_only =>
        "Quick Picker opened in copy-only mode. Choose a snippet, then paste it yourself; the picker does not request automatic insertion.",
        "Der Schnellwähler ist im Nur-Kopieren-Modus geöffnet. Wählen Sie ein Snippet und fügen Sie es selbst ein; der Wähler fordert kein automatisches Einfügen an.",
        "Quick Picker dibuka dalam mode salin saja. Pilih cuplikan, lalu tempel sendiri; picker tidak meminta penyisipan otomatis.";
    gnome_resume_title =>
        "GNOME setup is ready to resume",
        "Die GNOME-Einrichtung kann fortgesetzt werden",
        "Penyiapan GNOME siap dilanjutkan";
    gnome_resume_body =>
        "WayExpand's GNOME extension is prepared. If you have not logged out and back in since setup paused, do that first so GNOME can load it. Then continue IBus setup here; replacement consent and any administrator authorization are still reviewed separately.",
        "Die GNOME-Erweiterung von WayExpand ist vorbereitet. Falls Sie sich seit der Unterbrechung nicht ab- und wieder angemeldet haben, tun Sie das zuerst, damit GNOME sie laden kann. Setzen Sie dann hier die IBus-Einrichtung fort; die Zustimmung zur Ersetzung und eine etwaige Administrator-Freigabe werden weiterhin getrennt geprüft.",
        "Ekstensi GNOME WayExpand sudah disiapkan. Jika Anda belum keluar dan masuk lagi sejak penyiapan dijeda, lakukan itu dulu agar GNOME dapat memuatnya. Lalu lanjutkan penyiapan IBus di sini; persetujuan penggantian dan otorisasi administrator tetap ditinjau terpisah.";
    gnome_resume_button =>
        "Resume GNOME setup",
        "GNOME-Einrichtung fortsetzen",
        "Lanjutkan penyiapan GNOME";
    safe_fallback_matcher_passed =>
        "The local matcher produced the expected result. Automatic replacement has not been verified in a real application.",
        "Der lokale Abgleich lieferte das erwartete Ergebnis. Die automatische Ersetzung wurde in keiner echten Anwendung verifiziert.",
        "Pencocok lokal memberi hasil yang diharapkan. Penggantian otomatis belum diverifikasi di aplikasi nyata.";
    safe_fallback_nothing_verified =>
        "No real-application insertion has been verified. Use Quick Picker to copy a snippet and paste it yourself.",
        "Kein Einfügen in einer echten Anwendung wurde verifiziert. Kopieren Sie ein Snippet mit dem Schnellwähler und fügen Sie es selbst ein.",
        "Belum ada penyisipan di aplikasi nyata yang terverifikasi. Gunakan Quick Picker untuk menyalin cuplikan lalu tempel sendiri.";
    test_in_real_application =>
        "Test in a real application",
        "In einer echten Anwendung testen",
        "Uji di aplikasi nyata";
    review_setup_and_consent =>
        "Review setup and replacement consent",
        "Einrichtung und Zustimmung zur Ersetzung prüfen",
        "Tinjau penyiapan dan persetujuan penggantian";
    open_quick_picker_copy =>
        "Open Quick Picker (copy and paste)",
        "Schnellwähler öffnen (kopieren und einfügen)",
        "Buka Quick Picker (salin dan tempel)";
    gnome_input_source =>
        "GNOME input source",
        "GNOME-Eingabequelle",
        "Sumber input GNOME";
    gnome_source_ready =>
        "ready",
        "bereit",
        "siap";
    gnome_source_needs_setup =>
        "needs setup",
        "muss eingerichtet werden",
        "perlu disiapkan";
    gnome_source_selected =>
        "WayExpand selected",
        "WayExpand ausgewählt",
        "WayExpand dipilih";
    gnome_source_select =>
        "select WayExpand",
        "WayExpand auswählen",
        "pilih WayExpand";
    gnome_register_help =>
        "Continue guided setup to check dependencies, register the engine, and explain any required administrator authorization.",
        "Setzen Sie die geführte Einrichtung fort, um Abhängigkeiten zu prüfen, die Engine zu registrieren und eine nötige Administrator-Freigabe zu erklären.",
        "Lanjutkan penyiapan terpandu untuk memeriksa dependensi, mendaftarkan engine, dan menjelaskan otorisasi administrator yang diperlukan.";
    gnome_select_help =>
        "Choose WayExpand from GNOME’s keyboard/input-source menu. Your existing keyboard layouts remain available.",
        "Wählen Sie WayExpand im Tastatur- bzw. Eingabequellen-Menü von GNOME. Ihre bisherigen Tastaturbelegungen bleiben verfügbar.",
        "Pilih WayExpand dari menu papan ketik/sumber input GNOME. Tata letak papan ketik Anda tetap tersedia.";
    activate_wayexpand =>
        "Activate WayExpand",
        "WayExpand aktivieren",
        "Aktifkan WayExpand";
    gnome_selected_check_again =>
        "I selected it — check again",
        "Ausgewählt – erneut prüfen",
        "Sudah dipilih — periksa lagi";
    static_test_expansion_available =>
        "An enabled static expansion is available for the real-app test.",
        "Für den Test in einer echten Anwendung ist eine aktive statische Erweiterung vorhanden.",
        "Ekspansi statis yang aktif tersedia untuk uji di aplikasi nyata.";
    authorize_or_use_picker =>
        "Use the review action above to authorize compatibility replacement, or keep using Quick Picker for manual paste.",
        "Erlauben Sie die Kompatibilitäts-Ersetzung über die Prüfaktion oben, oder nutzen Sie weiter den Schnellwähler zum manuellen Einfügen.",
        "Gunakan tindakan tinjau di atas untuk mengizinkan penggantian kompatibilitas, atau tetap gunakan Quick Picker untuk menempel manual.";
    the_tested_application =>
        "the tested application",
        "der getesteten Anwendung",
        "aplikasi yang diuji";
    use_test_action_above =>
        "Use the test action above to verify this route in a real application.",
        "Prüfen Sie diesen Weg mit der Testaktion oben in einer echten Anwendung.",
        "Gunakan tindakan uji di atas untuk memverifikasi jalur ini di aplikasi nyata.";
}

texts! {
    non_atomic_setup_warning =>
        "Text replacement is not atomic: WayExpand checks and removes the trigger, then inserts the snippet separately. If you keep typing, your keys can interleave with the snippet and leave partial or corrupted text. Focus changes or backend failures can also leave text that cannot be rolled back. WayExpand will not retry an uncertain result. Headless tests do not certify physical-session target integrity; there is no accepted physical-session certification for this route.",
        "Die Textersetzung ist nicht atomar: WayExpand prüft und entfernt das Kürzel und fügt das Snippet danach separat ein. Wenn Sie weitertippen, können sich Ihre Tasten mit dem Snippet vermischen und unvollständigen oder beschädigten Text hinterlassen. Fokuswechsel oder Backend-Fehler können ebenfalls Text hinterlassen, der sich nicht zurücknehmen lässt. WayExpand wiederholt kein unsicheres Ergebnis. Headless-Tests zertifizieren nicht die Integrität des Zieltexts in einer echten Sitzung; für diesen Weg gibt es keine anerkannte Zertifizierung in einer echten Sitzung.",
        "Penggantian teks tidak atomik: WayExpand memeriksa dan menghapus pemicu, lalu menyisipkan cuplikan secara terpisah. Jika Anda terus mengetik, tombol Anda dapat bercampur dengan cuplikan dan meninggalkan teks yang terpotong atau rusak. Perpindahan fokus atau kegagalan backend juga dapat meninggalkan teks yang tidak dapat dikembalikan. WayExpand tidak akan mengulang hasil yang tidak pasti. Uji headless tidak menyertifikasi keutuhan teks target di sesi fisik; belum ada sertifikasi sesi fisik yang diterima untuk jalur ini.";
    setup_title =>
        "Set up WayExpand",
        "WayExpand einrichten",
        "Siapkan WayExpand";
    setup_checks =>
        "Setup checks",
        "Einrichtungsprüfungen",
        "Pemeriksaan penyiapan";
    setup_check_tools =>
        "Required tools and desktop dependencies",
        "Benötigte Werkzeuge und Desktop-Abhängigkeiten",
        "Alat dan dependensi desktop yang diperlukan";
    setup_check_component =>
        "IBus engine component installed",
        "IBus-Engine-Komponente installiert",
        "Komponen engine IBus terpasang";
    setup_check_registered =>
        "IBus engine registered",
        "IBus-Engine registriert",
        "Engine IBus terdaftar";
    setup_check_gnome_registered =>
        "GNOME input source registered",
        "GNOME-Eingabequelle registriert",
        "Sumber input GNOME terdaftar";
    setup_check_gnome_active =>
        "WayExpand selected in GNOME",
        "WayExpand in GNOME ausgewählt",
        "WayExpand dipilih di GNOME";
    setup_check_consent =>
        "Replacement authorization",
        "Freigabe der Ersetzung",
        "Otorisasi penggantian";
    setup_state_ready =>
        "Ready",
        "Bereit",
        "Siap";
    setup_state_warn =>
        "Action may be needed",
        "Eventuell ist etwas zu tun",
        "Mungkin perlu tindakan";
    setup_state_fail =>
        "Will be addressed during setup",
        "Wird bei der Einrichtung erledigt",
        "Akan ditangani saat penyiapan";
    setup_state_skip =>
        "Not required here",
        "Hier nicht nötig",
        "Tidak diperlukan di sini";
    setup_state_unknown =>
        "Checked during setup",
        "Wird bei der Einrichtung geprüft",
        "Diperiksa saat penyiapan";
    setup_step_prepare =>
        "Prepare WayExpand",
        "WayExpand vorbereiten",
        "Siapkan WayExpand";
    setup_step_prepare_detail =>
        "Use the installed files and create your configuration if needed.",
        "Die installierten Dateien verwenden und bei Bedarf Ihre Konfiguration anlegen.",
        "Gunakan berkas yang terpasang dan buat konfigurasi Anda bila perlu.";
    setup_step_activate =>
        "Activate and check the input engine",
        "Eingabe-Engine aktivieren und prüfen",
        "Aktifkan dan periksa engine input";
    setup_step_activate_detail =>
        "Then test a real expansion in a text field.",
        "Danach eine echte Erweiterung in einem Textfeld testen.",
        "Lalu uji ekspansi nyata di kolom teks.";
    setup_gnome_input =>
        "Register and select the IBus source",
        "IBus-Quelle registrieren und auswählen",
        "Daftarkan dan pilih sumber IBus";
    setup_gnome_input_detail =>
        "Keep your existing keyboard sources and layout in GNOME.",
        "Ihre bisherigen Eingabequellen und Tastaturbelegung in GNOME bleiben erhalten.",
        "Sumber papan ketik dan tata letak Anda di GNOME tetap dipertahankan.";
    setup_gnome_tracking =>
        "Enable GNOME window tracking",
        "GNOME-Fensterverfolgung aktivieren",
        "Aktifkan pelacakan jendela GNOME";
    setup_gnome_tracking_detail =>
        "Used for app-specific snippets; ordinary snippets do not depend on it. No administrator access is needed.",
        "Wird für anwendungsspezifische Snippets genutzt; gewöhnliche Snippets brauchen sie nicht. Keine Administratorrechte nötig.",
        "Dipakai untuk cuplikan khusus aplikasi; cuplikan biasa tidak bergantung padanya. Tidak perlu akses administrator.";
    setup_kde_input =>
        "Connect IBus to the KDE virtual keyboard",
        "IBus mit der virtuellen Tastatur von KDE verbinden",
        "Hubungkan IBus ke papan ketik virtual KDE";
    setup_kde_input_detail =>
        "WayExpand will preserve your configured keyboard layout.",
        "WayExpand behält Ihre eingestellte Tastaturbelegung bei.",
        "WayExpand akan mempertahankan tata letak papan ketik Anda.";
    setup_kde_tracking =>
        "Use KWin window tracking",
        "KWin-Fensterverfolgung verwenden",
        "Gunakan pelacakan jendela KWin";
    setup_tracking_detail =>
        "Used for app-specific snippets; ordinary snippets do not depend on it.",
        "Wird für anwendungsspezifische Snippets genutzt; gewöhnliche Snippets brauchen sie nicht.",
        "Dipakai untuk cuplikan khusus aplikasi; cuplikan biasa tidak bergantung padanya.";
    setup_other_input =>
        "Activate the IBus input source",
        "IBus-Eingabequelle aktivieren",
        "Aktifkan sumber input IBus";
    setup_other_input_detail =>
        "Your existing keyboard configuration is left in place.",
        "Ihre bisherige Tastaturkonfiguration bleibt unverändert.",
        "Konfigurasi papan ketik Anda tetap dibiarkan.";
    setup_other_tracking =>
        "Check application window tracking",
        "Fensterverfolgung für Anwendungen prüfen",
        "Periksa pelacakan jendela aplikasi";
    setup_unknown_tracking_detail =>
        "App-specific snippets need a supported window tracker; Diagnostics will show whether one is available.",
        "Anwendungsspezifische Snippets brauchen eine unterstützte Fensterverfolgung; die Diagnose zeigt, ob eine verfügbar ist.",
        "Cuplikan khusus aplikasi memerlukan pelacak jendela yang didukung; Diagnostik akan menunjukkan apakah tersedia.";
    setup_other_tracking_detail =>
        "App-specific snippets need a supported window tracker; ordinary snippets do not depend on it.",
        "Anwendungsspezifische Snippets brauchen eine unterstützte Fensterverfolgung; gewöhnliche Snippets brauchen sie nicht.",
        "Cuplikan khusus aplikasi memerlukan pelacak jendela yang didukung; cuplikan biasa tidak bergantung padanya.";
    setup_admin_needed =>
        "WayExpand needs one system-wide input-method registration. An administrator prompt may appear to install the shared registration and engine; this does not grant raw keyboard-device access. You can cancel if you prefer to install a distribution package instead.",
        "WayExpand braucht eine systemweite Registrierung als Eingabemethode. Eventuell erscheint eine Administrator-Abfrage, um die gemeinsame Registrierung und die Engine zu installieren; das gewährt keinen direkten Zugriff auf Tastaturgeräte. Sie können abbrechen, wenn Sie lieber ein Paket Ihrer Distribution installieren.",
        "WayExpand memerlukan satu pendaftaran metode input untuk seluruh sistem. Permintaan administrator mungkin muncul untuk memasang pendaftaran bersama dan engine; ini tidak memberi akses langsung ke perangkat papan ketik. Anda dapat membatalkan jika lebih suka memasang paket distribusi.";
    setup_admin_checkbox =>
        "Allow WayExpand to request administrator authorization for IBus registration if needed.",
        "WayExpand darf bei Bedarf eine Administrator-Freigabe für die IBus-Registrierung anfordern.",
        "Izinkan WayExpand meminta otorisasi administrator untuk pendaftaran IBus bila perlu.";
    setup_admin_not_needed =>
        "The shared input-method registration is already valid. No administrator authorization is needed for this step.",
        "Die gemeinsame Registrierung als Eingabemethode ist bereits gültig. Für diesen Schritt ist keine Administrator-Freigabe nötig.",
        "Pendaftaran metode input bersama sudah valid. Langkah ini tidak memerlukan otorisasi administrator.";
    setup_consent_checkbox =>
        "I understand the interleaving and partial-write risk and want to enable tracked-focus replacement.",
        "Ich verstehe das Risiko vermischter und unvollständiger Eingaben und möchte die fokusverfolgte Ersetzung aktivieren.",
        "Saya memahami risiko teks bercampur dan tertulis sebagian, dan ingin mengaktifkan penggantian dengan pelacakan fokus.";
    setup_consent_already =>
        "Tracked-focus text replacement is already enabled.",
        "Die fokusverfolgte Textersetzung ist bereits aktiviert.",
        "Penggantian teks dengan pelacakan fokus sudah aktif.";
    setup_continue =>
        "Continue setup",
        "Einrichtung fortsetzen",
        "Lanjutkan penyiapan";
}

texts! {
    setup_checking_ibus =>
        "Checking the IBus component, GNOME source, and replacement authorization…",
        "IBus-Komponente, GNOME-Quelle und Freigabe der Ersetzung werden geprüft…",
        "Memeriksa komponen IBus, sumber GNOME, dan otorisasi penggantian…";
    setup_preflight_complete =>
        "Setup preflight complete.",
        "Vorabprüfung der Einrichtung abgeschlossen.",
        "Pemeriksaan awal penyiapan selesai.";
    setup_waiting_for_routes =>
        "Checking available input routes. Setup will continue automatically when preflight finishes.",
        "Verfügbare Eingabewege werden geprüft. Die Einrichtung geht danach automatisch weiter.",
        "Memeriksa jalur input yang tersedia. Penyiapan berlanjut otomatis setelah pemeriksaan awal selesai.";
    setup_selecting_gnome =>
        "Selecting WayExpand in GNOME…",
        "WayExpand wird in GNOME ausgewählt…",
        "Memilih WayExpand di GNOME…";
    setup_gnome_selected_refreshing =>
        "WayExpand was selected; refreshing the GNOME checks.",
        "WayExpand wurde ausgewählt; die GNOME-Prüfungen werden aktualisiert.",
        "WayExpand telah dipilih; memperbarui pemeriksaan GNOME.";
    setup_needs_desktop_restart =>
        "Needs desktop restart: GNOME must load the installed extension after you log out and back in. Setup is not yet verified; resume it after the new session starts.",
        "Neustart der Sitzung nötig: GNOME lädt die installierte Erweiterung erst, nachdem Sie sich ab- und wieder angemeldet haben. Die Einrichtung ist noch nicht verifiziert; setzen Sie sie in der neuen Sitzung fort.",
        "Perlu memulai ulang desktop: GNOME memuat ekstensi yang terpasang setelah Anda keluar lalu masuk lagi. Penyiapan belum terverifikasi; lanjutkan setelah sesi baru dimulai.";
    setup_needs_permission =>
        "Needs permission: replacement is disabled. Review and explicitly authorize the non-atomic replacement behavior before expecting trigger expansion.",
        "Freigabe nötig: Die Ersetzung ist deaktiviert. Prüfen und erlauben Sie ausdrücklich die nicht atomare Ersetzung, bevor Kürzel erweitert werden können.",
        "Perlu izin: penggantian dinonaktifkan. Tinjau dan izinkan secara eksplisit perilaku penggantian non-atomik sebelum pemicu dapat diperluas.";
    setup_output_observed =>
        "Output activity observed. Check the target field and confirm the exact text before recording a result.",
        "Ausgabe beobachtet. Prüfen Sie das Zielfeld und bestätigen Sie den exakten Text, bevor Sie ein Ergebnis festhalten.",
        "Aktivitas keluaran teramati. Periksa kolom target dan konfirmasi teks yang tepat sebelum mencatat hasil.";
    setup_desktop_test_unverified =>
        "Desktop test was not verified. Inspect Diagnostics and the target field; do not retry uncertain output automatically.",
        "Der Desktop-Test wurde nicht verifiziert. Prüfen Sie die Diagnose und das Zielfeld; wiederholen Sie unsichere Ausgaben nicht automatisch.",
        "Uji desktop tidak terverifikasi. Periksa Diagnostik dan kolom target; jangan mengulang keluaran yang tidak pasti secara otomatis.";
    setup_configured_untested =>
        "Configured, untested: complete Desktop Integration Test to verify a real insertion in a target application.",
        "Eingerichtet, ungetestet: Schließen Sie den Desktop-Integrationstest ab, um ein echtes Einfügen in einer Zielanwendung zu verifizieren.",
        "Sudah dikonfigurasi, belum diuji: selesaikan Uji Integrasi Desktop untuk memverifikasi penyisipan nyata di aplikasi target.";
    setup_working =>
        "Working: a real insertion was confirmed in the tested application.",
        "Funktioniert: Ein echtes Einfügen wurde in der getesteten Anwendung bestätigt.",
        "Berfungsi: penyisipan nyata telah dikonfirmasi di aplikasi yang diuji.";
    setup_needs_action =>
        "Setup needs action. Review Diagnostics for the failing prerequisite.",
        "Die Einrichtung braucht Ihr Eingreifen. Die Diagnose zeigt die fehlende Voraussetzung.",
        "Penyiapan memerlukan tindakan. Tinjau Diagnostik untuk prasyarat yang gagal.";
    setup_checks_unreadable =>
        "Could not read IBus setup checks. Open Diagnostics, refresh the report, and try setup again.",
        "Die IBus-Einrichtungsprüfungen konnten nicht gelesen werden. Öffnen Sie die Diagnose, aktualisieren Sie den Bericht und starten Sie die Einrichtung erneut.",
        "Pemeriksaan penyiapan IBus tidak dapat dibaca. Buka Diagnostik, segarkan laporan, lalu coba penyiapan lagi.";
    setup_checks_incomplete =>
        "IBus setup checks were incomplete. Open Diagnostics and resolve the IBus registration check before continuing.",
        "Die IBus-Einrichtungsprüfungen waren unvollständig. Öffnen Sie die Diagnose und klären Sie die IBus-Registrierung, bevor Sie fortfahren.",
        "Pemeriksaan penyiapan IBus tidak lengkap. Buka Diagnostik dan selesaikan pemeriksaan pendaftaran IBus sebelum melanjutkan.";
    setup_review_stages =>
        "Review the setup stages and any required consent to continue.",
        "Prüfen Sie die Einrichtungsschritte und nötige Zustimmungen, um fortzufahren.",
        "Tinjau tahap penyiapan dan persetujuan yang diperlukan untuk melanjutkan.";
    setup_desktop_test_running =>
        "Desktop test running; type the selected trigger once in the confirmed disposable field.",
        "Desktop-Test läuft; tippen Sie das gewählte Kürzel einmal in das bestätigte Testfeld.",
        "Uji desktop berjalan; ketik pemicu yang dipilih sekali di kolom uji yang sudah dikonfirmasi.";
    setup_desktop_test_finished =>
        "Desktop test finished.",
        "Desktop-Test beendet.",
        "Uji desktop selesai.";
    setup_gnome_selected_first_run =>
        "WayExpand was selected in GNOME. Input-source configuration is updated; first-run setup still requires a real-app test.",
        "WayExpand wurde in GNOME ausgewählt. Die Eingabequellen sind aktualisiert; die Ersteinrichtung braucht noch einen Test in einer echten Anwendung.",
        "WayExpand telah dipilih di GNOME. Konfigurasi sumber input diperbarui; penyiapan pertama masih memerlukan uji di aplikasi nyata.";
    setup_gnome_selected =>
        "WayExpand was selected in GNOME. Input-source configuration is updated; real insertion remains unverified.",
        "WayExpand wurde in GNOME ausgewählt. Die Eingabequellen sind aktualisiert; echtes Einfügen ist weiterhin unverifiziert.",
        "WayExpand telah dipilih di GNOME. Konfigurasi sumber input diperbarui; penyisipan nyata masih belum terverifikasi.";
    setup_activity_uncorrelated =>
        "Activity observed but not correlated to the test trigger. Visually verify the exact target text before confirming.",
        "Aktivität beobachtet, aber nicht dem Testkürzel zugeordnet. Prüfen Sie den exakten Zieltext mit eigenen Augen, bevor Sie bestätigen.",
        "Aktivitas teramati tetapi tidak terkait dengan pemicu uji. Periksa teks target secara langsung sebelum mengonfirmasi.";
    setup_configured_unverified =>
        "Configured, unverified: complete the Desktop Integration Test and confirm the exact result in a real application. You can defer it, but WayExpand will not mark the route Working until confirmation.",
        "Eingerichtet, unverifiziert: Schließen Sie den Desktop-Integrationstest ab und bestätigen Sie das exakte Ergebnis in einer echten Anwendung. Sie können ihn aufschieben, aber bis dahin markiert WayExpand den Weg nicht als funktionierend.",
        "Sudah dikonfigurasi, belum terverifikasi: selesaikan Uji Integrasi Desktop dan konfirmasi hasil yang tepat di aplikasi nyata. Anda dapat menundanya, tetapi WayExpand tidak akan menandai jalur ini Berfungsi sampai saat itu.";
}

impl Strings {
    pub fn setup_unverified_state_unsaved(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("WayExpand is configured but unverified. The incomplete first-run state could not be saved, so the guide may not reopen automatically: {error}"),
            Language::German => format!("WayExpand ist eingerichtet, aber unverifiziert. Der offene Einrichtungszustand konnte nicht gespeichert werden, daher öffnet sich der Assistent eventuell nicht wieder von selbst: {error}"),
            Language::Indonesian => format!("WayExpand sudah dikonfigurasi tetapi belum terverifikasi. Status penggunaan pertama yang belum selesai tidak dapat disimpan, jadi panduan mungkin tidak terbuka lagi otomatis: {error}"),
        }
    }

    pub fn setup_unverified_fallback(&self, fallback: &str) -> String {
        match self.lang {
            Language::English => format!("{fallback} Configuration and service state were not enough to verify a real application insertion; check Diagnostics."),
            Language::German => format!("{fallback} Konfiguration und Dienststatus reichen nicht, um ein echtes Einfügen zu verifizieren; prüfen Sie die Diagnose."),
            Language::Indonesian => format!("{fallback} Konfigurasi dan status layanan tidak cukup untuk memverifikasi penyisipan di aplikasi nyata; periksa Diagnostik."),
        }
    }

    pub fn setup_could_not_check_stages(&self, detail: &str) -> String {
        match self.lang {
            Language::English => format!("Could not check the IBus setup stages: {detail}"),
            Language::German => {
                format!("Die IBus-Einrichtungsschritte konnten nicht geprüft werden: {detail}")
            }
            Language::Indonesian => format!("Tahap penyiapan IBus tidak dapat diperiksa: {detail}"),
        }
    }

    pub fn status_restarting_unit(&self, unit: &str) -> String {
        match self.lang {
            Language::English => format!("Restarting {unit}…"),
            Language::German => format!("{unit} wird neu gestartet…"),
            Language::Indonesian => format!("Memulai ulang {unit}…"),
        }
    }

    pub fn status_restart_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not start service restart: {error}"),
            Language::German => format!("Der Dienst konnte nicht neu gestartet werden: {error}"),
            Language::Indonesian => format!("Mulai ulang layanan tidak dapat dijalankan: {error}"),
        }
    }

    pub fn verification_in_application(&self, application: &str) -> String {
        match self.lang {
            Language::English => format!("Verified in tested application: {application}"),
            Language::German => format!("In getesteter Anwendung verifiziert: {application}"),
            Language::Indonesian => format!("Terverifikasi di aplikasi yang diuji: {application}"),
        }
    }

    pub fn status_real_app_test_state_unsaved(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!(
                "A real-app test is still required, but the pending first-run state could not be saved: {error}"
            ),
            Language::German => format!(
                "Ein Test in einer echten Anwendung ist weiterhin nötig, aber der offene Einrichtungszustand konnte nicht gespeichert werden: {error}"
            ),
            Language::Indonesian => format!(
                "Uji di aplikasi nyata masih diperlukan, tetapi status penggunaan pertama yang tertunda tidak dapat disimpan: {error}"
            ),
        }
    }

    pub fn status_picker_executable_missing(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not locate the Quick Picker executable: {error}"),
            Language::German => {
                format!("Das Programm des Schnellwählers wurde nicht gefunden: {error}")
            }
            Language::Indonesian => format!("Program Quick Picker tidak ditemukan: {error}"),
        }
    }

    pub fn status_picker_open_failed(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Could not open Quick Picker: {error}"),
            Language::German => format!("Der Schnellwähler konnte nicht geöffnet werden: {error}"),
            Language::Indonesian => format!("Quick Picker tidak dapat dibuka: {error}"),
        }
    }

    pub fn confirmed_in_application(&self, application: &str) -> String {
        match self.lang {
            Language::English => format!(
                "You confirmed the exact test replacement in {application}. This verifies that application and session only."
            ),
            Language::German => format!(
                "Sie haben die exakte Testersetzung in {application} bestätigt. Das verifiziert nur diese Anwendung und Sitzung."
            ),
            Language::Indonesian => format!(
                "Anda mengonfirmasi penggantian uji yang tepat di {application}. Ini hanya memverifikasi aplikasi dan sesi tersebut."
            ),
        }
    }

    pub fn gnome_source_state(&self, registration: &str, active: &str) -> String {
        match self.lang {
            Language::English => format!("Registration: {registration} · Active: {active}"),
            Language::German => format!("Registrierung: {registration} · Aktiv: {active}"),
            Language::Indonesian => format!("Pendaftaran: {registration} · Aktif: {active}"),
        }
    }

    pub fn expected_result(&self, replacement: &str) -> String {
        match self.lang {
            Language::English => format!("Expected result: {replacement}"),
            Language::German => format!("Erwartetes Ergebnis: {replacement}"),
            Language::Indonesian => format!("Hasil yang diharapkan: {replacement}"),
        }
    }
}

impl Strings {
    pub(crate) fn form_field_error(&self, error: crate::app::FormFieldError) -> &'static str {
        use crate::app::FormFieldError as E;
        match (self.lang, error) {
            (Language::English, E::InvalidName) => "Use a field name beginning with a lowercase letter, followed by lowercase letters, digits, or underscores (up to 64 characters).",
            (Language::German, E::InvalidName) => "Verwenden Sie einen Feldnamen, der mit einem Kleinbuchstaben beginnt und danach nur Kleinbuchstaben, Ziffern oder Unterstriche enthält (bis zu 64 Zeichen).",
            (Language::Indonesian, E::InvalidName) => "Gunakan nama kolom yang diawali huruf kecil, diikuti huruf kecil, angka, atau garis bawah (hingga 64 karakter).",
            (Language::English, E::UnsafeDefault) => "The default contains characters that cannot be represented safely in a form marker.",
            (Language::German, E::UnsafeDefault) => "Der Standardwert enthält Zeichen, die sich in einer Formularmarkierung nicht sicher darstellen lassen.",
            (Language::Indonesian, E::UnsafeDefault) => "Nilai bawaan berisi karakter yang tidak dapat dinyatakan dengan aman dalam penanda formulir.",
            (Language::English, E::InvalidChoices) => "Enter 2–32 safe dropdown choices separated by commas.",
            (Language::German, E::InvalidChoices) => "Geben Sie 2–32 sichere Auswahlwerte ein, durch Kommas getrennt.",
            (Language::Indonesian, E::InvalidChoices) => "Masukkan 2–32 pilihan dropdown yang aman, dipisahkan koma.",
            (Language::English, E::DefaultNotAChoice) => "The dropdown default must exactly match one of its choices.",
            (Language::German, E::DefaultNotAChoice) => "Der Standardwert muss genau einem der Auswahlwerte entsprechen.",
            (Language::Indonesian, E::DefaultNotAChoice) => "Nilai bawaan dropdown harus sama persis dengan salah satu pilihannya.",
        }
    }
}
