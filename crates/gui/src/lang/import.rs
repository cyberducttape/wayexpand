use super::{Language, Strings};

impl Strings {
    pub fn import_preview_summary(
        &self,
        fully_migrated: usize,
        with_warnings: usize,
        unsupported: usize,
    ) -> String {
        match self.lang {
            Language::English => format!(
                "Migration preview: {fully_migrated} fully migrated, {with_warnings} with warnings, {unsupported} unsupported"
            ),
            Language::Indonesian => format!(
                "Pratinjau migrasi: {fully_migrated} termigrasi penuh, {with_warnings} dengan peringatan, {unsupported} tidak didukung"
            ),
            Language::German => format!(
                "Importvorschau: {fully_migrated} vollständig importiert, {with_warnings} mit Warnungen, {unsupported} nicht unterstützt"
            ),
        }
    }

    pub fn search_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Search triggers, descriptions, tags, and categories (Ctrl+F)",
            Language::Indonesian => "Cari pemicu, deskripsi, tag, dan kategori (Ctrl+F)",
            Language::German => {
                "Trigger, Beschreibungen, Schlagwörter und Kategorien durchsuchen (Strg+F)"
            }
        }
    }

    pub fn command_backed_summary(&self) -> &'static str {
        match self.lang {
            Language::English => "Command-backed snippet",
            Language::Indonesian => "Cuplikan berbasis perintah",
            Language::German => "Befehlsgestütztes Snippet",
        }
    }

    pub fn app_filter_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Leave empty for every app. Add an app to limit this snippet to that application.",
            Language::Indonesian => "Kosongkan untuk semua aplikasi. Tambahkan aplikasi untuk membatasi cuplikan ini ke aplikasi tersebut.",
            Language::German => "Leer lassen für alle Apps. Eine App hinzufügen, um dieses Snippet auf diese Anwendung zu begrenzen.",
        }
    }

    pub fn app_filter_advanced_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Advanced syntax: bare values or app_id_exact:<id> match one normalized desktop app ID. app_id_glob:<pattern> and title_contains:<text> intentionally use weaker matching. Filters currently require the KWin window tracker.",
            Language::Indonesian => "Sintaks lanjutan: nilai biasa atau app_id_exact:<id> cocok dengan satu ID aplikasi desktop yang dinormalisasi. app_id_glob:<pattern> dan title_contains:<text> sengaja menggunakan pencocokan yang lebih lemah. Filter saat ini memerlukan pelacak jendela KWin.",
            Language::German => "Erweiterte Syntax: Einzelwerte oder app_id_exact:<id> gleichen eine normalisierte Desktop-App-ID exakt ab. app_id_glob:<Muster> und title_contains:<Text> verwenden absichtlich schwächeres Matching. Filter benötigen derzeit den KWin-Fenster-Tracker.",
        }
    }

    pub fn app_filter_advanced(&self) -> &'static str {
        match self.lang {
            Language::English => "Advanced matching syntax",
            Language::Indonesian => "Sintaks pencocokan lanjutan",
            Language::German => "Erweiterte Matching-Syntax",
        }
    }

    pub fn matching(&self) -> &'static str {
        match self.lang {
            Language::English => "Matching",
            Language::Indonesian => "Pencocokan",
            Language::German => "Erkennung",
        }
    }

    pub fn aliases(&self) -> &'static str {
        match self.lang {
            Language::English => "Aliases",
            Language::Indonesian => "Alias",
            Language::German => "Aliase",
        }
    }

    pub fn add_alias(&self) -> &'static str {
        match self.lang {
            Language::English => "Add alias…",
            Language::Indonesian => "Tambah alias…",
            Language::German => "Alias hinzufügen…",
        }
    }

    pub fn remove_alias(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove alias",
            Language::Indonesian => "Hapus alias",
            Language::German => "Alias entfernen",
        }
    }

    pub fn add_tag(&self) -> &'static str {
        match self.lang {
            Language::English => "Add tag…",
            Language::Indonesian => "Tambah tag…",
            Language::German => "Tag hinzufügen…",
        }
    }

    pub fn remove_tag(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove tag",
            Language::Indonesian => "Hapus tag",
            Language::German => "Tag entfernen",
        }
    }

    pub fn add_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Add app id…",
            Language::Indonesian => "Tambah ID aplikasi…",
            Language::German => "App-ID hinzufügen…",
        }
    }

    pub fn remove_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove app",
            Language::Indonesian => "Hapus aplikasi",
            Language::German => "App entfernen",
        }
    }

    pub fn add_argument(&self) -> &'static str {
        match self.lang {
            Language::English => "+ Add argument",
            Language::Indonesian => "+ Tambah argumen",
            Language::German => "+ Argument hinzufügen",
        }
    }

    pub fn move_up(&self) -> &'static str {
        match self.lang {
            Language::English => "Move up",
            Language::Indonesian => "Pindah ke atas",
            Language::German => "Nach oben",
        }
    }

    pub fn move_down(&self) -> &'static str {
        match self.lang {
            Language::English => "Move down",
            Language::Indonesian => "Pindah ke bawah",
            Language::German => "Nach unten",
        }
    }

    pub fn remove_argument(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove argument",
            Language::Indonesian => "Hapus argumen",
            Language::German => "Argument entfernen",
        }
    }

    pub fn environment(&self) -> &'static str {
        match self.lang {
            Language::English => "Environment",
            Language::Indonesian => "Lingkungan",
            Language::German => "Umgebung",
        }
    }

    pub fn environment_minimal(&self) -> &'static str {
        match self.lang {
            Language::English => "Minimal",
            Language::Indonesian => "Minimal",
            Language::German => "Minimal",
        }
    }

    pub fn environment_inherit(&self) -> &'static str {
        match self.lang {
            Language::English => "Inherit",
            Language::Indonesian => "Warisi",
            Language::German => "Übernehmen",
        }
    }

    pub fn environment_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Minimal passes only a safe PATH and the variables listed below; Inherit passes the editor's whole environment.",
            Language::Indonesian => "Minimal hanya meneruskan PATH yang aman dan variabel yang tercantum di bawah; Warisi meneruskan seluruh lingkungan editor.",
            Language::German => "Minimal übergibt nur einen sicheren PATH und die unten aufgeführten Variablen; Übernehmen übergibt die gesamte Umgebung des Editors.",
        }
    }

    pub fn pass_environment(&self) -> &'static str {
        match self.lang {
            Language::English => "Pass environment variables",
            Language::Indonesian => "Teruskan variabel lingkungan",
            Language::German => "Umgebungsvariablen übergeben",
        }
    }

    pub fn add_environment_variable(&self) -> &'static str {
        match self.lang {
            Language::English => "+ Add environment variable",
            Language::Indonesian => "+ Tambah variabel lingkungan",
            Language::German => "+ Umgebungsvariable hinzufügen",
        }
    }

    pub fn remove_environment_variable(&self) -> &'static str {
        match self.lang {
            Language::English => "Remove environment variable",
            Language::Indonesian => "Hapus variabel lingkungan",
            Language::German => "Umgebungsvariable entfernen",
        }
    }

    pub fn no_changes_to_save(&self) -> &'static str {
        match self.lang {
            Language::English => "No unsaved changes",
            Language::Indonesian => "Tidak ada perubahan yang belum disimpan",
            Language::German => "Keine ungespeicherten Änderungen",
        }
    }

    pub fn duplicate_needs_selection(&self) -> &'static str {
        match self.lang {
            Language::English => "Select a snippet to duplicate it",
            Language::Indonesian => "Pilih cuplikan untuk menduplikasinya",
            Language::German => "Wählen Sie ein Snippet zum Duplizieren",
        }
    }

    pub fn template_variable_description(&self, variable: &str) -> &'static str {
        match (self.lang, variable) {
            (Language::English, "{{date}}") => "UTC date",
            (Language::Indonesian, "{{date}}") => "Tanggal UTC",
            (Language::German, "{{date}}") => "Datum (UTC)",
            (Language::English, "{{time}}") => "UTC time",
            (Language::Indonesian, "{{time}}") => "Waktu UTC",
            (Language::German, "{{time}}") => "Uhrzeit (UTC)",
            (Language::English, "{{datetime}}") => "UTC date and time",
            (Language::Indonesian, "{{datetime}}") => "Tanggal dan waktu UTC",
            (Language::German, "{{datetime}}") => "Datum und Uhrzeit (UTC)",
            (Language::English, "{{local_date}}") => "Date in the system time zone",
            (Language::Indonesian, "{{local_date}}") => "Tanggal dalam zona waktu sistem",
            (Language::German, "{{local_date}}") => "Datum (Systemzeitzone)",
            (Language::English, "{{local_time}}") => "Time in the system time zone",
            (Language::Indonesian, "{{local_time}}") => "Waktu dalam zona waktu sistem",
            (Language::German, "{{local_time}}") => "Uhrzeit (Systemzeitzone)",
            (Language::English, "{{local_datetime}}") => {
                "Date and time in the system time zone, with its UTC offset"
            }
            (Language::Indonesian, "{{local_datetime}}") => {
                "Tanggal dan waktu dalam zona waktu sistem, dengan selisih UTC"
            }
            (Language::German, "{{local_datetime}}") => {
                "Datum und Uhrzeit (Systemzeitzone, mit UTC-Versatz)"
            }
            (Language::English, "{{date+1d}}") => {
                "Tomorrow's date (also -1d, +1w; date, time, datetime and local_ forms; d/w/h/m units)"
            }
            (Language::Indonesian, "{{date+1d}}") => {
                "Tanggal besok (juga -1d, +1w; date, time, datetime, dan bentuk local_; satuan d/w/h/m)"
            }
            (Language::German, "{{date+1d}}") => {
                "Morgiges Datum (auch -1d, +1w; date, time, datetime und local_-Varianten; Einheiten d/w/h/m)"
            }
            (Language::English, "{{cursor}}") => {
                "Place the cursor here after expanding (libei and wlroots backends)"
            }
            (Language::Indonesian, "{{cursor}}") => {
                "Letakkan kursor di sini setelah ekspansi (backend libei dan wlroots)"
            }
            (Language::German, "{{cursor}}") => {
                "Setzt den Cursor nach dem Erweitern hierher (Backends libei und wlroots)"
            }
            (Language::English, "{{username}}") => "Current user",
            (Language::Indonesian, "{{username}}") => "Pengguna saat ini",
            (Language::German, "{{username}}") => "Aktueller Benutzer",
            (Language::English, "{{hostname}}") => "Local hostname",
            (Language::Indonesian, "{{hostname}}") => "Nama host lokal",
            (Language::German, "{{hostname}}") => "Lokaler Rechnername",
            (Language::English, "{{unix_timestamp}}") => "Unix timestamp",
            (Language::Indonesian, "{{unix_timestamp}}") => "Stempel waktu Unix",
            (Language::German, "{{unix_timestamp}}") => "Unix-Zeitstempel",
            (Language::English, "{{newline}}") => "Line break",
            (Language::Indonesian, "{{newline}}") => "Baris baru",
            (Language::German, "{{newline}}") => "Zeilenumbruch",
            (Language::English, "{{tab}}") => "Tab character",
            (Language::Indonesian, "{{tab}}") => "Karakter tab",
            (Language::German, "{{tab}}") => "Tabulatorzeichen",
            _ => "",
        }
    }

    pub fn description_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "What is this snippet for? (shown in the list)",
            Language::Indonesian => "Untuk apa cuplikan ini? (ditampilkan di daftar)",
            Language::German => "Wofür ist dieses Snippet? (in der Liste angezeigt)",
        }
    }

    pub fn command_not_run_by_gui(&self) -> &'static str {
        match self.lang {
            Language::English => "This command is not run by the GUI.",
            Language::Indonesian => "Perintah ini tidak dijalankan oleh GUI.",
            Language::German => "Dieser Befehl wird nicht von der GUI ausgeführt.",
        }
    }

    pub fn unknown_desktop(&self) -> &'static str {
        match self.lang {
            Language::English => "Linux desktop",
            Language::Indonesian => "Desktop Linux",
            Language::German => "Linux-Desktop",
        }
    }

    pub fn try_live_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Matcher preview",
            Language::Indonesian => "Pratinjau pencocok",
            Language::German => "Matcher-Vorschau",
        }
    }

    pub fn try_live_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Tests matching only — not desktop integration. It does not test capture, focus, portals, sensitive fields, or insertion; command snippets are not run.",
            Language::Indonesian => "Hanya menguji pencocokan — bukan integrasi desktop. Tidak menguji tangkapan, fokus, portal, kolom sensitif, atau penyisipan; cuplikan perintah tidak dijalankan.",
            Language::German => "Testet nur Matching — keine Desktop-Integration. Erfassung, Fokus, Portale, sensible Felder und Einfügen werden nicht getestet; Befehls-Snippets laufen nicht.",
        }
    }

    pub fn try_live_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Type a trigger and watch it expand…",
            Language::Indonesian => "Ketik pemicu dan lihat hasil ekspansinya…",
            Language::German => "Tippen Sie einen Auslöser und sehen Sie zu…",
        }
    }

    pub fn try_live_stats(&self, expansions: usize, saved: usize) -> String {
        match self.lang {
            Language::English => format!(
                "{expansions} expansion{} · {saved} keystrokes saved",
                if expansions == 1 { "" } else { "s" }
            ),
            // Indonesian nouns do not inflect for plural.
            Language::Indonesian => {
                format!("{expansions} ekspansi · {saved} ketukan tombol dihemat")
            }
            Language::German => format!(
                "{expansions} Erweiterung{} · {saved} Tastenanschläge gespart",
                if expansions == 1 { "" } else { "en" }
            ),
        }
    }

    pub fn clear(&self) -> &'static str {
        match self.lang {
            Language::English => "Clear",
            Language::Indonesian => "Bersihkan",
            Language::German => "Leeren",
        }
    }

    pub fn picker_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Search snippets…",
            Language::Indonesian => "Cari cuplikan…",
            Language::German => "Snippets suchen…",
        }
    }

    pub fn picker_no_results(&self) -> &'static str {
        match self.lang {
            Language::English => "No matching snippets",
            Language::Indonesian => "Tidak ada cuplikan yang cocok",
            Language::German => "Keine passenden Snippets",
        }
    }

    pub fn picker_footer_daemon(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "↑↓ select · Enter types it into the app you were using · Esc cancels"
            }
            Language::Indonesian => {
                "↑↓ pilih · Enter mengetikkannya ke aplikasi yang sedang Anda gunakan · Esc membatalkan"
            }
            Language::German => {
                "↑↓ auswählen · Enter tippt es in die vorherige App · Esc bricht ab"
            }
        }
    }

    pub fn picker_footer_clipboard(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Can't type into the previous app (no daemon or window tracking) · Enter copies the snippet to the clipboard"
            }
            Language::Indonesian => {
                "Tidak dapat mengetik ke aplikasi sebelumnya (tidak ada daemon atau pelacakan jendela) · Enter menyalin cuplikan ke papan klip"
            }
            Language::German => {
                "Tippen in die vorherige App nicht möglich (kein Dienst oder Fenster-Tracking) · Enter kopiert das Snippet in die Zwischenablage"
            }
        }
    }

    pub fn picker_footer_identity_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Exact window identity: unavailable · Quick Picker typing: clipboard-only"
            }
            Language::Indonesian => {
                "Identitas jendela yang tepat: tidak tersedia · Pengetikan Quick Picker: hanya papan klip"
            }
            Language::German => {
                "Exakte Fensteridentität: nicht verfügbar · Quick Picker-Eingabe: nur Zwischenablage"
            }
        }
    }

    pub fn picker_copied(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Copied {trigger} · paste it with Ctrl+V, then press Esc")
            }
            Language::Indonesian => {
                format!("{trigger} disalin · tempel dengan Ctrl+V, lalu tekan Esc")
            }
            Language::German => {
                format!("{trigger} kopiert · mit Strg+V einfügen, dann Esc drücken")
            }
        }
    }

    pub fn picker_inserting(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Inserting {trigger}…"),
            Language::Indonesian => format!("Menyisipkan {trigger}…"),
            Language::German => format!("{trigger} wird eingefügt …"),
        }
    }

    pub fn picker_waiting_for_focus(&self) -> &'static str {
        match self.lang {
            Language::English => "Waiting for the original window to regain focus safely.",
            Language::Indonesian => "Menunggu jendela asal kembali fokus dengan aman.",
            Language::German => {
                "Warten, bis das ursprüngliche Fenster sicher wieder fokussiert ist."
            }
        }
    }

    pub fn picker_insert_failed(&self) -> &'static str {
        match self.lang {
            Language::English => "Could not safely return to the original window.",
            Language::Indonesian => "Tidak dapat kembali ke jendela asal dengan aman.",
            Language::German => {
                "Das ursprüngliche Fenster konnte nicht sicher wieder fokussiert werden."
            }
        }
    }

    pub fn picker_copy_instead(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy instead",
            Language::Indonesian => "Salin saja",
            Language::German => "Stattdessen kopieren",
        }
    }

    pub fn cancel(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancel",
            Language::Indonesian => "Batal",
            Language::German => "Abbrechen",
        }
    }

    pub fn turn_on(&self) -> &'static str {
        match self.lang {
            Language::English => "Turn on WayExpand",
            Language::Indonesian => "Aktifkan WayExpand",
            Language::German => "WayExpand einschalten",
        }
    }

    pub fn turning_on(&self) -> &'static str {
        match self.lang {
            Language::English => "Turning on…",
            Language::Indonesian => "Mengaktifkan…",
            Language::German => "Wird eingeschaltet…",
        }
    }

    pub fn turn_on_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Runs `wayexpand setup --yes`: enables the safest detected input path for this desktop. It never grants raw keyboard access; your desktop may ask for permission.",
            Language::Indonesian => "Menjalankan `wayexpand setup --yes`: mengaktifkan jalur masukan teraman yang terdeteksi untuk desktop ini. Ini tidak pernah memberikan akses papan ketik mentah; desktop Anda mungkin meminta izin.",
            Language::German => "Führt `wayexpand setup --yes` aus: aktiviert den sichersten erkannten Eingabeweg. Es wird nie roher Tastaturzugriff gewährt; der Desktop fragt eventuell nach Erlaubnis.",
        }
    }

    pub fn status_setup_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Turning on WayExpand…",
            Language::Indonesian => "Mengaktifkan WayExpand…",
            Language::German => "WayExpand wird eingeschaltet…",
        }
    }

    pub fn status_setup_cancelling(&self) -> &'static str {
        match self.lang {
            Language::English => "Cancelling setup…",
            Language::Indonesian => "Membatalkan penyiapan…",
            Language::German => "Einrichtung wird abgebrochen…",
        }
    }

    pub fn status_setup_done(&self) -> &'static str {
        match self.lang {
            Language::English => "WayExpand is on · try a normal text field in an app you use; see Desktop details for route limits",
            Language::Indonesian => "WayExpand aktif · coba kolom teks biasa di aplikasi yang Anda gunakan; lihat Detail desktop untuk batasan rute",
            Language::German => "WayExpand ist aktiv · probieren Sie ein normales Textfeld in einer verwendeten App; siehe Desktop-Details für Einschränkungen",
        }
    }

    pub fn status_setup_failed(&self, detail: &str) -> String {
        match (self.lang, detail.is_empty()) {
            (Language::English, true) => "Setup failed; run `wayexpand doctor` for details".into(),
            (Language::Indonesian, true) => {
                "Penyiapan gagal; jalankan `wayexpand doctor` untuk detailnya".into()
            }
            (Language::German, true) => {
                "Einrichtung fehlgeschlagen; Details mit `wayexpand doctor`".into()
            }
            (Language::English, false) => format!("Setup failed: {detail}"),
            (Language::Indonesian, false) => format!("Penyiapan gagal: {detail}"),
            (Language::German, false) => format!("Einrichtung fehlgeschlagen: {detail}"),
        }
    }

    pub fn onboarding_turn_on_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Turn on WayExpand",
            Language::Indonesian => "Aktifkan WayExpand",
            Language::German => "WayExpand einschalten",
        }
    }

    pub fn onboarding_turn_on_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Enables the safest input path WayExpand detects on this desktop. Your desktop may ask for permission.",
            Language::Indonesian => "Mengaktifkan jalur masukan teraman yang dideteksi WayExpand di desktop ini. Desktop Anda mungkin meminta izin.",
            Language::German => "Aktiviert den sichersten Eingabeweg, den WayExpand auf diesem Desktop erkennt. Der Desktop fragt eventuell nach Erlaubnis.",
        }
    }

    pub fn onboarding_snippet_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Add your first snippet",
            Language::Indonesian => "Tambahkan cuplikan pertama Anda",
            Language::German => "Erstes Snippet anlegen",
        }
    }

    pub fn onboarding_snippet_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Start with a test snippet, write your own, or bring your Espanso library.",
            Language::Indonesian => "Mulai dengan cuplikan uji, tulis sendiri, atau bawa pustaka Espanso Anda.",
            Language::German => "Beginnen Sie mit einem Test-Snippet, schreiben Sie ein eigenes oder übernehmen Sie Ihre Espanso-Bibliothek.",
        }
    }

    pub fn onboarding_details_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Desktop details",
            Language::Indonesian => "Detail desktop",
            Language::German => "Desktop-Details",
        }
    }
}
