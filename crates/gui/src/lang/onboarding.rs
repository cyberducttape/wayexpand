//! First-run guide and guided-setup text.

use super::{Language, Strings};

texts! {
    outcome_working =>
        "Working in tested application",
        "Funktioniert in der getesteten Anwendung",
        "Berfungsi di aplikasi yang diuji";
    outcome_configured_awaiting_test =>
        "Configured, awaiting test",
        "Konfiguriert, Test ausstehend",
        "Dikonfigurasi, menunggu pengujian";
    outcome_safe_fallback =>
        "Automatic replacement unavailable · copy and paste",
        "Automatische Ersetzung nicht verfügbar · kopieren und einfügen",
        "Penggantian otomatis tidak tersedia · salin dan tempel";
    compatibility_replacement_active =>
        "Compatibility replacement is enabled (non-atomic). A test covers only the named app in this session; focus changes or typing during insertion can leave partial text. Quick Picker copy-and-paste remains available.",
        "Die Kompatibilitäts-Ersetzung ist aktiviert (nicht atomar). Ein Test gilt nur für die genannte Anwendung in dieser Sitzung; Fokuswechsel oder Tippen während des Einfügens können unvollständigen Text hinterlassen. Kopieren und Einfügen über den Schnellwähler bleibt verfügbar.",
        "Penggantian kompatibilitas aktif (non-atomik). Pengujian hanya berlaku untuk aplikasi yang disebutkan dalam sesi ini; perubahan fokus atau mengetik saat penyisipan dapat meninggalkan teks sebagian. Salin-tempel melalui Quick Picker tetap tersedia.";
    outcome_setup_incomplete =>
        "Setup incomplete",
        "Einrichtung unvollständig",
        "Penyiapan belum selesai";
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
    gnome_resume_cancel_button =>
        "Cancel continuation",
        "Fortsetzung abbrechen",
        "Batalkan kelanjutan";
    gnome_resume_cancelled =>
        "GNOME setup continuation cancelled. No snippets or integration settings were removed.",
        "Die GNOME-Einrichtungsfortsetzung wurde abgebrochen. Snippets und Integrationseinstellungen wurden nicht entfernt.",
        "Kelanjutan penyiapan GNOME dibatalkan. Cuplikan dan pengaturan integrasi tidak dihapus.";
    gnome_resume_stale_title =>
        "A GNOME setup continuation is not active in this session",
        "Eine GNOME-Einrichtungsfortsetzung ist in dieser Sitzung nicht aktiv",
        "Kelanjutan penyiapan GNOME tidak aktif dalam sesi ini";
    gnome_resume_stale_body =>
        "This saved continuation is expired or belongs to another desktop session, so WayExpand will not resume it automatically. Cancel it to remove the stale marker; this does not change your snippets or integration settings.",
        "Diese gespeicherte Fortsetzung ist abgelaufen oder gehört zu einer anderen Desktop-Sitzung. WayExpand setzt sie daher nicht automatisch fort. Brechen Sie sie ab, um den veralteten Marker zu entfernen; Snippets und Integrationseinstellungen bleiben unverändert.",
        "Kelanjutan tersimpan ini kedaluwarsa atau milik sesi desktop lain, jadi WayExpand tidak akan melanjutkannya otomatis. Batalkan untuk menghapus penanda lama; cuplikan dan pengaturan integrasi tidak berubah.";
    test_in_real_application =>
        "Test expansion",
        "Textersetzung testen",
        "Uji ekspansi teks";
    review_setup_and_consent =>
        "Allow text replacement…",
        "Textersetzung erlauben…",
        "Izinkan penggantian teks…";
    onboarding_set_up_recommended_ibus =>
        "Set up recommended desktop integration…",
        "Empfohlene Desktop-Integration einrichten…",
        "Siapkan integrasi desktop yang disarankan…";
    open_quick_picker_copy =>
        "Open Quick Picker (copy and paste)",
        "Schnellwähler öffnen (kopieren und einfügen)",
        "Buka Quick Picker (salin dan tempel)";
    gnome_register_help =>
        "Continue guided setup to check what is missing and explain any administrator approval it needs.",
        "Setzen Sie die geführte Einrichtung fort, um fehlende Voraussetzungen zu prüfen und nötige Administratorfreigaben zu erklären.",
        "Lanjutkan penyiapan terpandu untuk memeriksa kebutuhan yang belum terpenuhi dan menjelaskan persetujuan administrator yang diperlukan.";
    gnome_select_help =>
        "Choose WayExpand from GNOME’s keyboard/input-source menu. Your existing keyboard layouts remain available.",
        "Wählen Sie WayExpand im Tastatur- bzw. Eingabequellen-Menü von GNOME. Ihre bisherigen Tastaturbelegungen bleiben verfügbar.",
        "Pilih WayExpand dari menu papan ketik/sumber input GNOME. Tata letak papan ketik Anda tetap tersedia.";
    authorize_or_use_picker =>
        "WayExpand needs permission to remove a typed trigger and replace it with your snippet. If typing or focus changes during that operation, the text can be left incomplete. Allow replacement for this desktop, or keep using Quick Picker to copy and paste.",
        "WayExpand benötigt die Erlaubnis, ein getipptes Kürzel zu entfernen und durch Ihr Snippet zu ersetzen. Wenn währenddessen getippt wird oder sich der Fokus ändert, kann unvollständiger Text zurückbleiben. Erlauben Sie die Ersetzung für diesen Desktop oder verwenden Sie weiterhin den Schnellwähler zum Kopieren und Einfügen.",
        "WayExpand memerlukan izin untuk menghapus pemicu yang diketik lalu menggantinya dengan cuplikan Anda. Jika ada pengetikan atau perubahan fokus selama proses, teks bisa tertinggal sebagian. Izinkan penggantian untuk desktop ini, atau tetap gunakan Quick Picker untuk menyalin dan menempel.";
    the_tested_application =>
        "the tested application",
        "der getesteten Anwendung",
        "aplikasi yang diuji";
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
        "Show technical checks",
        "Technische Prüfungen anzeigen",
        "Tampilkan pemeriksaan teknis";
    setup_check_tools =>
        "Required tools and desktop dependencies",
        "Benötigte Werkzeuge und Desktop-Abhängigkeiten",
        "Alat dan dependensi desktop yang diperlukan";
    setup_check_component =>
        "IBus engine component installed",
        "IBus-Engine-Komponente installiert",
        "Komponen engine IBus terpasang";
    setup_check_manifest =>
        "IBus manifest points to the WayExpand engine",
        "Das IBus-Manifest verweist auf die WayExpand-Engine",
        "Manifest IBus mengarah ke mesin WayExpand";
    setup_check_registered =>
        "IBus engine registered",
        "IBus-Engine registriert",
        "Engine IBus terdaftar";
    setup_check_desktop =>
        "Desktop routes input to IBus",
        "Der Desktop leitet Eingaben an IBus weiter",
        "Desktop mengarahkan input ke IBus";
    setup_check_selected =>
        "WayExpand is the selected IBus engine",
        "WayExpand ist die ausgewählte IBus-Engine",
        "WayExpand adalah mesin IBus yang dipilih";
    setup_check_engine =>
        "WayExpand engine is running",
        "Die WayExpand-Engine läuft",
        "Mesin WayExpand sedang berjalan";
    setup_check_gnome_bridge =>
        "GNOME Shell integration is available",
        "Die GNOME-Shell-Integration ist verfügbar",
        "Integrasi GNOME Shell tersedia";
    setup_check_manifest_scope =>
        "Shared IBus engine uses a protected system executable",
        "Die gemeinsame IBus-Engine nutzt eine geschützte Systemdatei",
        "Engine IBus bersama memakai executable sistem yang terlindungi";
    setup_check_action =>
        "Next:",
        "Nächster Schritt:",
        "Berikutnya:";
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
        "Activate WayExpand and check keyboard input",
        "WayExpand aktivieren und Tastatureingabe prüfen",
        "Aktifkan WayExpand dan periksa input papan ketik";
    setup_step_activate_detail =>
        "Then test a real expansion in a text field.",
        "Danach eine echte Erweiterung in einem Textfeld testen.",
        "Lalu uji ekspansi nyata di kolom teks.";
    setup_gnome_input =>
        "Add WayExpand to your GNOME keyboard list",
        "WayExpand zur GNOME-Tastaturliste hinzufügen",
        "Tambahkan WayExpand ke daftar papan ketik GNOME";
    setup_gnome_input_detail =>
        "Your existing keyboard sources and layout are preserved.",
        "Ihre bisherigen Eingabequellen und Tastaturbelegung bleiben erhalten.",
        "Sumber papan ketik dan tata letak Anda tetap dipertahankan.";
    setup_gnome_tracking =>
        "Connect WayExpand to GNOME",
        "WayExpand mit GNOME verbinden",
        "Hubungkan WayExpand ke GNOME";
    setup_gnome_tracking_detail =>
        "Needed to activate and verify WayExpand in GNOME; it also enables app-specific snippets. No administrator access is needed.",
        "Wird benötigt, um WayExpand in GNOME zu aktivieren und zu prüfen; außerdem ermöglicht es anwendungsspezifische Snippets. Keine Administratorrechte nötig.",
        "Diperlukan untuk mengaktifkan dan memverifikasi WayExpand di GNOME; juga mendukung cuplikan khusus aplikasi. Tidak perlu akses administrator.";
    setup_kde_input =>
        "Connect WayExpand to the KDE keyboard",
        "WayExpand mit der KDE-Tastatur verbinden",
        "Hubungkan WayExpand ke papan ketik KDE";
    setup_kde_input_detail =>
        "WayExpand will preserve your configured keyboard layout.",
        "WayExpand behält Ihre eingestellte Tastaturbelegung bei.",
        "WayExpand akan mempertahankan tata letak papan ketik Anda.";
    setup_kde_tracking =>
        "Enable app-specific snippets",
        "Anwendungsspezifische Snippets aktivieren",
        "Aktifkan cuplikan khusus aplikasi";
    setup_tracking_detail =>
        "Only needed for snippets restricted to specific apps; ordinary snippets do not depend on it.",
        "Nur für Snippets nötig, die auf bestimmte Apps beschränkt sind; gewöhnliche Snippets brauchen es nicht.",
        "Hanya diperlukan untuk cuplikan yang dibatasi ke aplikasi tertentu; cuplikan biasa tidak memerlukannya.";
    setup_other_input =>
        "Activate WayExpand for typing",
        "WayExpand für die Texteingabe aktivieren",
        "Aktifkan WayExpand untuk mengetik";
    setup_other_input_detail =>
        "Your existing keyboard configuration is left in place.",
        "Ihre bisherige Tastaturkonfiguration bleibt unverändert.",
        "Konfigurasi papan ketik Anda tetap dibiarkan.";
    setup_other_tracking =>
        "Check app-specific snippet support",
        "Unterstützung für app-spezifische Snippets prüfen",
        "Periksa dukungan cuplikan khusus aplikasi";
    setup_unknown_tracking_detail =>
        "App-specific snippets need desktop focus support; Diagnostics will show whether your desktop provides it.",
        "App-spezifische Snippets benötigen Fokusunterstützung durch den Desktop; die Diagnose zeigt, ob sie verfügbar ist.",
        "Cuplikan khusus aplikasi memerlukan dukungan fokus dari desktop; Diagnostik akan menunjukkan apakah tersedia.";
    setup_other_tracking_detail =>
        "App-specific snippets need desktop focus support; ordinary snippets do not depend on it.",
        "App-spezifische Snippets benötigen Fokusunterstützung durch den Desktop; gewöhnliche Snippets brauchen sie nicht.",
        "Cuplikan khusus aplikasi memerlukan dukungan fokus dari desktop; cuplikan biasa tidak memerlukannya.";
    setup_admin_needed =>
        "WayExpand needs a shared keyboard integration. An administrator prompt may appear to register it; this does not grant raw keyboard-device access. You can cancel and install a distribution package instead.",
        "WayExpand benötigt eine gemeinsame Tastaturintegration. Möglicherweise erscheint eine Administratorabfrage zur Registrierung; dadurch erhält WayExpand keinen direkten Zugriff auf Tastaturgeräte. Sie können abbrechen und stattdessen ein Distributionspaket installieren.",
        "WayExpand memerlukan integrasi papan ketik bersama. Permintaan administrator mungkin muncul untuk mendaftarkannya; ini tidak memberi akses ke perangkat papan ketik mentah. Anda dapat membatalkan dan memasang paket distribusi.";
    setup_admin_checkbox =>
        "Allow WayExpand to ask for administrator approval to register the keyboard integration if needed.",
        "WayExpand darf bei Bedarf eine Administratorfreigabe zur Registrierung der Tastaturintegration anfordern.",
        "Izinkan WayExpand meminta persetujuan administrator untuk mendaftarkan integrasi papan ketik bila diperlukan.";
    setup_admin_not_needed =>
        "The keyboard integration is already registered. No administrator approval is needed for this step.",
        "Die Tastaturintegration ist bereits registriert. Für diesen Schritt ist keine Administratorfreigabe nötig.",
        "Integrasi papan ketik sudah terdaftar. Langkah ini tidak memerlukan persetujuan administrator.";
    setup_consent_checkbox =>
        "I understand the interleaving and partial-write risk and want to enable tracked-focus replacement.",
        "Ich verstehe das Risiko vermischter und unvollständiger Eingaben und möchte die fokusverfolgte Ersetzung aktivieren.",
        "Saya memahami risiko teks bercampur dan tertulis sebagian, dan ingin mengaktifkan penggantian dengan pelacakan fokus.";
    setup_continue =>
        "Continue setup",
        "Einrichtung fortsetzen",
        "Lanjutkan penyiapan";
}

impl Strings {
    /// Describe the actual persisted consent mode. In particular, do not
    /// label `untracked-focus` as `tracked-focus`: users need to know which
    /// non-atomic replacement policy is already active before continuing.
    pub(crate) fn setup_consent_already(&self, mode: wayexpand_core::CompatibilityMode) -> String {
        let mode = self.compatibility_mode_option(mode);
        match self.lang {
            Language::English => {
                format!("Non-atomic text replacement is already enabled. Focus mode: {mode}.")
            }
            Language::German => format!(
                "Die nicht atomare Textersetzung ist bereits aktiviert. Fokusmodus: {mode}."
            ),
            Language::Indonesian => {
                format!("Penggantian teks non-atomik sudah aktif. Mode fokus: {mode}.")
            }
        }
    }
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
    setup_replacement_consent_needed =>
        "Replacement consent needed: automatic replacement is off. Review and explicitly allow non-atomic replacement before expecting triggers to expand.",
        "Zustimmung zur Ersetzung nötig: Die automatische Ersetzung ist deaktiviert. Prüfen und erlauben Sie ausdrücklich die nicht atomare Ersetzung, bevor Kürzel erweitert werden können.",
        "Persetujuan penggantian diperlukan: penggantian otomatis nonaktif. Tinjau dan izinkan secara eksplisit penggantian non-atomik sebelum pemicu dapat diperluas.";
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
        "A test field will open. Type the trigger there once; do not press Enter. If automatic checking is unavailable, WayExpand will ask you to confirm what appeared.",
        "Ein Testfeld wird geöffnet. Tippen Sie dort einmal das Kürzel und drücken Sie nicht die Eingabetaste. Wenn die automatische Prüfung nicht verfügbar ist, fragt WayExpand, was angezeigt wurde.",
        "Kolom uji akan terbuka. Ketik pemicu sekali di sana; jangan tekan Enter. Jika pemeriksaan otomatis tidak tersedia, WayExpand akan meminta Anda mengonfirmasi hasilnya.";
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

// The three-step first-run guide: connect, create, test.
texts! {
    step_connect_title =>
        "Prepare your desktop",
        "Desktop vorbereiten",
        "Siapkan desktop Anda";
    step_create_title =>
        "Create a snippet",
        "Snippet anlegen",
        "Buat cuplikan";
    step_test_title =>
        "Test it",
        "Ausprobieren",
        "Uji coba";
    first_run_checking =>
        "Checking this desktop…",
        "Dieser Desktop wird geprüft …",
        "Memeriksa desktop ini…";
    first_run_connected =>
        "Desktop typing: {detail}",
        "Texteingabe am Desktop: {detail}",
        "Pengetikan desktop: {detail}";
    first_run_check_again =>
        "Check again",
        "Erneut prüfen",
        "Periksa lagi";
    first_run_snippet_ready =>
        "Test snippet saved: {trigger} → “{replacement}”. Step 3 checks that it appears in a text field.",
        "Testsnippet gespeichert: {trigger} → „{replacement}“. In Schritt 3 wird geprüft, ob der Text im Eingabefeld erscheint.",
        "Cuplikan uji tersimpan: {trigger} → “{replacement}”. Langkah 3 memeriksa apakah teks muncul di kolom.";
    first_run_test_needs_connection =>
        "Finish step 1 first. Until then you can copy snippets with Quick Picker and paste them yourself.",
        "Schließen Sie zuerst Schritt 1 ab. Bis dahin können Sie Snippets mit dem Schnellwähler kopieren und selbst einfügen.",
        "Selesaikan langkah 1 dulu. Sementara itu Anda bisa menyalin cuplikan dengan Quick Picker dan menempelkannya sendiri.";
    first_run_test_needs_snippet =>
        "Create a snippet in step 2 first.",
        "Legen Sie zuerst in Schritt 2 ein Snippet an.",
        "Buat cuplikan di langkah 2 dulu.";
    first_run_test_help =>
        "A harmless test field opens. Type the trigger once. WayExpand checks the text there when possible; otherwise it asks you to confirm the result. This verifies only the tested field and desktop session.",
        "Ein harmloses Testfeld wird geöffnet. Tippen Sie dort einmal das Kürzel. WayExpand prüft den Text nach Möglichkeit automatisch und fragt sonst nach Ihrer Bestätigung. Geprüft werden nur dieses Feld und diese Sitzung.",
        "Kolom uji yang aman akan terbuka. Ketik pemicu sekali. WayExpand memeriksa teks jika memungkinkan; jika tidak, WayExpand meminta konfirmasi Anda. Pemeriksaan hanya berlaku untuk kolom dan sesi ini.";
    first_run_try_inside =>
        "Try the trigger inside WayExpand first",
        "Auslöser zuerst in WayExpand ausprobieren",
        "Coba pemicu di dalam WayExpand dulu";
    first_run_trigger_works_inside =>
        "Matcher preview: this rule matches locally. It does not verify keyboard input or text replacement in an application.",
        "Matcher-Vorschau: Diese Regel passt lokal. Tastatureingabe und Textersetzung in einer Anwendung werden dadurch nicht verifiziert.",
        "Pratinjau matcher: aturan ini cocok secara lokal. Ini tidak memverifikasi input keyboard atau penggantian teks di aplikasi.";
    first_run_troubleshooting_details =>
        "Troubleshooting details",
        "Details zur Fehlerbehebung",
        "Detail pemecahan masalah";
    setup_no_route_available =>
        "No compatible input route is available yet. Diagnostics shows the missing prerequisite; fix it, then try again.",
        "Noch ist kein passender Eingabeweg verfügbar. Die Diagnose zeigt die fehlende Voraussetzung; beheben Sie sie und versuchen Sie es erneut.",
        "Belum ada jalur masukan yang cocok. Diagnostik menunjukkan prasyarat yang kurang; perbaiki, lalu coba lagi.";
    setup_preflight_stopped =>
        "The IBus setup check stopped unexpectedly. Open Diagnostics and try again.",
        "Die IBus-Einrichtungsprüfung wurde unerwartet beendet. Öffnen Sie die Diagnose und versuchen Sie es erneut.",
        "Pemeriksaan penyiapan IBus berhenti tiba-tiba. Buka Diagnostik dan coba lagi.";
}

// The optional Quick Picker shortcut step.
texts! {
    step_shortcut_title =>
        "Open snippets from any app (optional)",
        "Snippets in jeder App öffnen (optional)",
        "Buka cuplikan dari aplikasi mana pun (opsional)";
    shortcut_help =>
        "Give the Quick Picker a keyboard shortcut. It can insert directly only when the destination is verified and the route supports atomic insertion. Otherwise, when policy allows, it copies the snippet so you can paste it yourself.",
        "Geben Sie dem Schnellwähler ein Tastenkürzel. Direktes Einfügen ist nur möglich, wenn das Ziel verifiziert ist und der Eingabeweg atomare Einfügung unterstützt. Andernfalls wird das Snippet – sofern die Richtlinie es erlaubt – kopiert, damit Sie es selbst einfügen.",
        "Beri Quick Picker pintasan papan ketik. Penyisipan langsung hanya tersedia jika tujuan terverifikasi dan jalur mendukung penyisipan atomik. Jika tidak, dan kebijakan mengizinkan, cuplikan disalin agar Anda dapat menempelkannya sendiri.";
    shortcut_use_keys =>
        "Use {keys}",
        "{keys} verwenden",
        "Pakai {keys}";
    shortcut_ready =>
        "Press {keys} in any app to open the Quick Picker.",
        "Drücken Sie {keys} in einer beliebigen App, um den Schnellwähler zu öffnen.",
        "Tekan {keys} di aplikasi mana pun untuk membuka Quick Picker.";
    shortcut_manual =>
        "This desktop does not let WayExpand set shortcuts. Add one in its keyboard settings that runs: wayexpand-gui --picker",
        "Auf diesem Desktop kann WayExpand keine Tastenkürzel setzen. Legen Sie in den Tastatureinstellungen eines an, das Folgendes ausführt: wayexpand-gui --picker",
        "Desktop ini tidak mengizinkan WayExpand mengatur pintasan. Tambahkan di pengaturan papan ketiknya pintasan yang menjalankan: wayexpand-gui --picker";
    shortcut_change_hint =>
        "Change it later in your desktop's keyboard settings.",
        "Ändern lässt es sich später in den Tastatureinstellungen des Desktops.",
        "Ubah nanti di pengaturan papan ketik desktop Anda.";
    shortcut_setting =>
        "Setting the Quick Picker shortcut…",
        "Tastenkürzel für den Schnellwähler wird gesetzt …",
        "Mengatur pintasan Quick Picker…";
    shortcut_set_done =>
        "Quick Picker shortcut set.",
        "Tastenkürzel für den Schnellwähler gesetzt.",
        "Pintasan Quick Picker diatur.";
}
