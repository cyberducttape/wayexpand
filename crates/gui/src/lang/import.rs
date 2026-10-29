use super::{Language, Strings};

impl Strings {
    pub fn pack_review_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Review a local snippet pack",
            Language::Indonesian => "Tinjau paket cuplikan lokal",
            Language::German => "Lokales Snippet-Paket prüfen",
        }
    }

    pub fn review_packs(&self) -> &'static str {
        match self.lang {
            Language::English => "Review local packs…",
            Language::Indonesian => "Tinjau paket lokal…",
            Language::German => "Lokale Pakete prüfen…",
        }
    }

    pub fn pack_review_source(&self) -> &'static str {
        match self.lang {
            Language::English => "Pack directory from a local catalog or Git checkout",
            Language::Indonesian => "Direktori paket dari katalog lokal atau checkout Git",
            Language::German => "Paketverzeichnis aus lokalem Katalog oder Git-Checkout",
        }
    }

    pub fn pack_review_button(&self) -> &'static str {
        match self.lang {
            Language::English => "Review pack",
            Language::Indonesian => "Tinjau paket",
            Language::German => "Paket prüfen",
        }
    }

    pub fn pack_install_button(&self) -> &'static str {
        match self.lang {
            Language::English => "Install reviewed pack",
            Language::Indonesian => "Pasang paket yang ditinjau",
            Language::German => "Geprüftes Paket installieren",
        }
    }

    pub fn pack_install_update(&self) -> &'static str {
        match self.lang {
            Language::English => "Install reviewed update",
            Language::Indonesian => "Pasang pembaruan yang ditinjau",
            Language::German => "Geprüftes Update installieren",
        }
    }

    pub fn pack_operation_running(&self) -> &'static str {
        match self.lang {
            Language::English => "Checking pack…",
            Language::Indonesian => "Memeriksa paket…",
            Language::German => "Paket wird geprüft…",
        }
    }

    pub fn pack_no_review_record(&self) -> &'static str {
        match self.lang {
            Language::English => "No review record is included. Treat this pack as unreviewed.",
            Language::Indonesian => "Tidak ada catatan tinjauan. Anggap paket ini belum ditinjau.",
            Language::German => "Kein Prüfvermerk enthalten. Dieses Paket gilt als ungeprüft.",
        }
    }

    pub fn pack_review_record(&self, reviewer: &str, date: &str, summary: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Review record: {reviewer} · {date} · {summary} (metadata only)")
            }
            Language::Indonesian => {
                format!("Catatan tinjauan: {reviewer} · {date} · {summary} (hanya metadata)")
            }
            Language::German => {
                format!("Prüfvermerk: {reviewer} · {date} · {summary} (nur Metadaten)")
            }
        }
    }

    pub fn pack_capability_disclosure(&self, capabilities: &[String]) -> String {
        let values = if capabilities.is_empty() {
            "none".to_owned()
        } else {
            capabilities.join(", ")
        };
        match self.lang {
            Language::English => format!("Required capabilities: {values}"),
            Language::Indonesian => format!("Kemampuan yang dibutuhkan: {values}"),
            Language::German => format!("Benötigte Fähigkeiten: {values}"),
        }
    }

    pub fn pack_action_counts(&self, commands: u64, hotkeys: u64) -> String {
        match self.lang {
            Language::English => {
                format!("Executable entries: {commands} commands · {hotkeys} hotkeys")
            }
            Language::Indonesian => {
                format!("Entri yang dapat dijalankan: {commands} perintah · {hotkeys} hotkey")
            }
            Language::German => {
                format!("Ausführbare Einträge: {commands} Befehle · {hotkeys} Hotkeys")
            }
        }
    }

    pub fn pack_broker_actions(&self, actions: &[String]) -> String {
        match self.lang {
            Language::English => format!("Named broker actions requested: {}", actions.join(", ")),
            Language::Indonesian => {
                format!("Aksi broker bernama yang diminta: {}", actions.join(", "))
            }
            Language::German => format!("Angeforderte Broker-Aktionen: {}", actions.join(", ")),
        }
    }

    pub fn pack_declared_capabilities(&self, capabilities: &[String]) -> String {
        match self.lang {
            Language::English => format!(
                "Publisher-declared capabilities: {}",
                capabilities.join(", ")
            ),
            Language::Indonesian => format!(
                "Kemampuan yang dinyatakan penerbit: {}",
                capabilities.join(", ")
            ),
            Language::German => format!(
                "Vom Herausgeber deklarierte Fähigkeiten: {}",
                capabilities.join(", ")
            ),
        }
    }

    pub fn pack_minimum_version(&self, version: &str) -> String {
        match self.lang {
            Language::English => format!("Requires WayExpand {version} or newer"),
            Language::Indonesian => format!("Memerlukan WayExpand {version} atau lebih baru"),
            Language::German => format!("Benötigt WayExpand {version} oder neuer"),
        }
    }

    pub fn pack_source_file(&self, file: &str) -> String {
        match self.lang {
            Language::English => format!("Source file: {file}"),
            Language::Indonesian => format!("Berkas sumber: {file}"),
            Language::German => format!("Quelldatei: {file}"),
        }
    }

    pub fn pack_snippets(&self, count: usize) -> String {
        match self.lang {
            Language::English => format!("Review snippet contents ({count})"),
            Language::Indonesian => format!("Tinjau isi cuplikan ({count})"),
            Language::German => format!("Snippet-Inhalte prüfen ({count})"),
        }
    }

    pub fn pack_allow_actions_consent(&self) -> &'static str {
        match self.lang {
            Language::English => "I reviewed these executable entries and allow them to become active. Installation does not run them.",
            Language::Indonesian => "Saya telah meninjau entri yang dapat dijalankan dan mengizinkannya aktif. Pemasangan tidak menjalankannya.",
            Language::German => "Ich habe die ausführbaren Einträge geprüft und erlaube ihre Aktivierung. Die Installation führt sie nicht aus.",
        }
    }

    pub fn pack_install_blocked(&self) -> &'static str {
        match self.lang {
            Language::English => "Resolve policy requirements and review action consent above.",
            Language::Indonesian => "Penuhi kebijakan dan persetujuan aksi di atas.",
            Language::German => "Richtlinienanforderungen und Aktionsfreigabe oben erfüllen.",
        }
    }

    pub fn pack_review_truncated(&self) -> &'static str {
        match self.lang {
            Language::English => "Some content is too large to display fully here. Review it in a terminal before installing.",
            Language::Indonesian => "Sebagian konten terlalu besar untuk ditampilkan penuh. Tinjau melalui terminal sebelum memasang.",
            Language::German => "Einige Inhalte sind hier zu groß für die vollständige Anzeige. Vor der Installation im Terminal prüfen.",
        }
    }

    pub fn pack_diff_required(&self) -> &'static str {
        match self.lang {
            Language::English => "The update diff could not be reviewed, so installation is disabled. Retry the review or inspect the update in a terminal.",
            Language::Indonesian => "Diff pembaruan tidak dapat ditinjau, jadi pemasangan dinonaktifkan. Ulangi peninjauan atau periksa pembaruan melalui terminal.",
            Language::German => "Das Update-Diff konnte nicht geprüft werden; die Installation ist deaktiviert. Prüfung wiederholen oder das Update im Terminal ansehen.",
        }
    }

    pub fn pack_signature_required(&self) -> &'static str {
        match self.lang {
            Language::English => "Organization policy requires a signature verified by its trusted signer list.",
            Language::Indonesian => "Kebijakan organisasi mewajibkan tanda tangan dari daftar penanda tangan tepercaya.",
            Language::German => "Die Organisationsrichtlinie verlangt eine Signatur aus der vertrauenswürdigen Signaturliste.",
        }
    }

    pub fn pack_signature_unverified(&self) -> &'static str {
        match self.lang {
            Language::English => "The signature could not be verified. Installation may be refused by current policy.",
            Language::Indonesian => "Tanda tangan tidak dapat diverifikasi. Kebijakan saat ini mungkin menolak pemasangan.",
            Language::German => "Die Signatur konnte nicht geprüft werden. Die Richtlinie kann die Installation ablehnen.",
        }
    }

    pub fn pack_unsigned_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Unsigned pack: publisher and review text are self-asserted metadata.",
            Language::Indonesian => "Paket tanpa tanda tangan: penerbit dan teks tinjauan adalah metadata yang dinyatakan sendiri.",
            Language::German => "Unsigniertes Paket: Herausgeber und Prüftext sind nicht bestätigte Metadaten.",
        }
    }

    pub fn pack_denied_by_policy(&self) -> &'static str {
        match self.lang {
            Language::English => "Organization policy does not allow this pack ID.",
            Language::Indonesian => "Kebijakan organisasi tidak mengizinkan ID paket ini.",
            Language::German => "Die Organisationsrichtlinie erlaubt diese Paket-ID nicht.",
        }
    }

    pub fn pack_commands_denied(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Organization policy disables command-backed entries in this pack."
            }
            Language::Indonesian => {
                "Kebijakan organisasi menonaktifkan entri berbasis perintah dalam paket ini."
            }
            Language::German => {
                "Die Organisationsrichtlinie deaktiviert befehlsbasierte Einträge in diesem Paket."
            }
        }
    }

    pub fn pack_hotkeys_denied(&self) -> &'static str {
        match self.lang {
            Language::English => "Organization policy disables hotkeys in this pack.",
            Language::Indonesian => "Kebijakan organisasi menonaktifkan hotkey dalam paket ini.",
            Language::German => "Die Organisationsrichtlinie deaktiviert Hotkeys in diesem Paket.",
        }
    }

    pub fn pack_diff_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Changes from installed version",
            Language::Indonesian => "Perubahan dari versi terpasang",
            Language::German => "Änderungen zur installierten Version",
        }
    }

    pub fn pack_install_complete(&self) -> &'static str {
        match self.lang {
            Language::English => "Pack installed and daemon reload requested",
            Language::Indonesian => "Paket terpasang dan pemuatan ulang daemon diminta",
            Language::German => "Paket installiert und Neuladen des Daemons angefordert",
        }
    }

    pub fn pack_install_result(&self, output: &str) -> String {
        match self.lang {
            Language::English => format!("Install result: {output}"),
            Language::Indonesian => format!("Hasil pemasangan: {output}"),
            Language::German => format!("Installationsergebnis: {output}"),
        }
    }

    pub fn form_builder_title(&self) -> &'static str {
        match self.lang {
            Language::English => "Build a form field",
            Language::Indonesian => "Buat kolom formulir",
            Language::German => "Formularfeld erstellen",
        }
    }

    pub fn form_builder_name(&self) -> &'static str {
        match self.lang {
            Language::English => "Field name",
            Language::Indonesian => "Nama kolom",
            Language::German => "Feldname",
        }
    }

    pub fn form_builder_default(&self) -> &'static str {
        match self.lang {
            Language::English => "Default",
            Language::Indonesian => "Nilai awal",
            Language::German => "Standardwert",
        }
    }

    pub fn form_builder_default_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "optional text / first choice",
            Language::Indonesian => "teks opsional / pilihan pertama",
            Language::German => "optionaler Text / erste Auswahl",
        }
    }

    pub fn form_builder_choices(&self) -> &'static str {
        match self.lang {
            Language::English => "Choices",
            Language::Indonesian => "Pilihan",
            Language::German => "Auswahlwerte",
        }
    }

    pub fn form_builder_required(&self) -> &'static str {
        match self.lang {
            Language::English => "Required",
            Language::Indonesian => "Wajib",
            Language::German => "Erforderlich",
        }
    }

    pub fn form_builder_text(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert text field",
            Language::Indonesian => "Sisipkan kolom teks",
            Language::German => "Textfeld einfügen",
        }
    }

    pub fn form_builder_choice(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert dropdown",
            Language::Indonesian => "Sisipkan daftar pilihan",
            Language::German => "Auswahlliste einfügen",
        }
    }

    pub fn form_builder_date(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert date picker",
            Language::Indonesian => "Sisipkan pemilih tanggal",
            Language::German => "Datumsauswahl einfügen",
        }
    }

    pub fn form_builder_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Names use lowercase letters, digits, and underscores. Choices are comma-separated; edit the inserted marker for length rules or more options.",
            Language::Indonesian => "Nama menggunakan huruf kecil, angka, dan garis bawah. Pilihan dipisahkan koma; edit penanda untuk aturan panjang atau pilihan lainnya.",
            Language::German => "Namen verwenden Kleinbuchstaben, Ziffern und Unterstriche. Auswahlwerte sind durch Kommas getrennt; für Längenregeln oder weitere Werte den Marker bearbeiten.",
        }
    }
}

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
            (Language::English, "{{local_date:%A %-d %B %Y}}") => {
                "Formatted date, e.g. Friday 9 October 2026. Codes: %Y year, %m month, %B/%b month name, %d day (%-d without zero), %A/%a weekday, %F 2026-10-09. Works on any date or time variable, also with +1d"
            }
            (Language::Indonesian, "{{local_date:%A %-d %B %Y}}") => {
                "Tanggal berformat, mis. Friday 9 October 2026. Kode: %Y tahun, %m bulan, %B/%b nama bulan, %d hari (%-d tanpa nol), %A/%a hari, %F 2026-10-09. Berlaku untuk variabel tanggal atau waktu apa pun, juga dengan +1d"
            }
            (Language::German, "{{local_date:%A %-d %B %Y}}") => {
                "Formatiertes Datum, z. B. Friday 9 October 2026. Codes: %Y Jahr, %m Monat, %B/%b Monatsname, %d Tag (%-d ohne Null), %A/%a Wochentag, %F 2026-10-09. Für jede Datums- oder Zeitvariable, auch mit +1d"
            }
            (Language::English, "{{local_time:%-I:%M %p}}") => {
                "Formatted time, e.g. 2:05 PM. Codes: %H 24-hour, %I 12-hour, %M minute, %S second, %p AM/PM, %:z UTC offset"
            }
            (Language::Indonesian, "{{local_time:%-I:%M %p}}") => {
                "Waktu berformat, mis. 2:05 PM. Kode: %H 24 jam, %I 12 jam, %M menit, %S detik, %p AM/PM, %:z selisih UTC"
            }
            (Language::German, "{{local_time:%-I:%M %p}}") => {
                "Formatierte Uhrzeit, z. B. 2:05 PM. Codes: %H 24 Stunden, %I 12 Stunden, %M Minute, %S Sekunde, %p AM/PM, %:z UTC-Versatz"
            }
            (Language::English, "{{uuid}}") => "A new random UUID each time",
            (Language::Indonesian, "{{uuid}}") => "UUID acak baru setiap kali",
            (Language::German, "{{uuid}}") => "Jedes Mal eine neue zufällige UUID",
            (Language::English, "{{uuid:id}}") => {
                "One random UUID, repeated wherever {{uuid:id}} appears in this expansion; use other names for more"
            }
            (Language::Indonesian, "{{uuid:id}}") => {
                "Satu UUID acak, diulang di mana pun {{uuid:id}} muncul dalam ekspansi ini; gunakan nama lain untuk lebih banyak"
            }
            (Language::German, "{{uuid:id}}") => {
                "Eine zufällige UUID, wiederholt überall, wo {{uuid:id}} in dieser Erweiterung steht; weitere Namen für weitere UUIDs"
            }
            (Language::English, "{{random:hex:16}}") => {
                "16 random characters; change the number (1-256) or the kind: hex, digits, lower, alpha, alnum"
            }
            (Language::Indonesian, "{{random:hex:16}}") => {
                "16 karakter acak; ubah angkanya (1-256) atau jenisnya: hex, digits, lower, alpha, alnum"
            }
            (Language::German, "{{random:hex:16}}") => {
                "16 zufällige Zeichen; Zahl (1-256) oder Art ändern: hex, digits, lower, alpha, alnum"
            }
            (Language::English, "{{random:password:20}}") => {
                "A random 20-character password: letters, digits and -_. without look-alikes"
            }
            (Language::Indonesian, "{{random:password:20}}") => {
                "Kata sandi acak 20 karakter: huruf, angka, dan -_. tanpa karakter yang mirip"
            }
            (Language::German, "{{random:password:20}}") => {
                "Zufälliges Passwort mit 20 Zeichen: Buchstaben, Ziffern und -_. ohne Verwechslungsgefahr"
            }
            (Language::English, "{{random:A|B|C}}") => "One of the options, picked at random (2-32 options)",
            (Language::Indonesian, "{{random:A|B|C}}") => "Salah satu pilihan, dipilih acak (2-32 pilihan)",
            (Language::German, "{{random:A|B|C}}") => "Eine der Optionen, zufällig gewählt (2-32 Optionen)",
            (Language::English, "{{field:name}}") => {
                "Ask for a value when the snippet expands; {{field:name=default}} prefills it. Typed forms need an input-method route; `wayexpand new` works everywhere"
            }
            (Language::Indonesian, "{{field:name}}") => {
                "Minta nilai saat cuplikan diekspansi; {{field:name=default}} mengisinya terlebih dahulu. Formulir yang diketik memerlukan rute input-method; `wayexpand new` berfungsi di mana saja"
            }
            (Language::German, "{{field:name}}") => {
                "Fragt beim Erweitern nach einem Wert; {{field:name=default}} füllt ihn vor. Getippte Formulare brauchen eine Input-Method-Route; `wayexpand new` funktioniert überall"
            }
            (Language::English, "{{field:name|required|min=2|max=80}}") => "Required text input with minimum and maximum character counts",
            (Language::Indonesian, "{{field:name|required|min=2|max=80}}") => "Isian teks wajib dengan batas minimum dan maksimum karakter",
            (Language::German, "{{field:name|required|min=2|max=80}}") => "Pflichtfeld mit Mindest- und Höchstlänge",
            (Language::English, "{{choice:name=A|B}}") => "A labelled drop-down with 2-32 options",
            (Language::Indonesian, "{{choice:name=A|B}}") => "Daftar pilihan berlabel dengan 2-32 opsi",
            (Language::German, "{{choice:name=A|B}}") => "Beschriftete Auswahlliste mit 2-32 Optionen",
            (Language::English, "{{choice:severity=Low|High;default=High}}") => "A drop-down with a preselected option",
            (Language::Indonesian, "{{choice:severity=Low|High;default=High}}") => "Daftar pilihan dengan opsi awal",
            (Language::German, "{{choice:severity=Low|High;default=High}}") => "Auswahlliste mit vorausgewählter Option",
            (Language::English, "{{datefield:date=today}}") => "Choose a calendar date, defaulting to today",
            (Language::Indonesian, "{{datefield:date=today}}") => "Pilih tanggal kalender, dengan nilai awal hari ini",
            (Language::German, "{{datefield:date=today}}") => "Kalenderdatum auswählen, standardmäßig heute",
            (Language::English, "{{{{ }}}}") => {
                "Literal {{ and }}, for Ansible, Helm or GitHub Actions syntax"
            }
            (Language::Indonesian, "{{{{ }}}}") => {
                "{{ dan }} apa adanya, untuk sintaks Ansible, Helm, atau GitHub Actions"
            }
            (Language::German, "{{{{ }}}}") => {
                "Wörtliches {{ und }}, für Ansible-, Helm- oder GitHub-Actions-Syntax"
            }
            _ => "",
        }
    }

    pub fn template_variable_group(&self, group: crate::TemplateVariableGroup) -> &'static str {
        use crate::TemplateVariableGroup::*;
        match (self.lang, group) {
            (Language::English, DateTime) => "Date and time",
            (Language::Indonesian, DateTime) => "Tanggal dan waktu",
            (Language::German, DateTime) => "Datum und Uhrzeit",
            (Language::English, Random) => "Unique and random",
            (Language::Indonesian, Random) => "Unik dan acak",
            (Language::German, Random) => "Eindeutig und zufällig",
            (Language::English, Fields) => "Fill in when expanding",
            (Language::Indonesian, Fields) => "Isi saat ekspansi",
            (Language::German, Fields) => "Beim Erweitern ausfüllen",
            (Language::English, Text) => "Text and system",
            (Language::Indonesian, Text) => "Teks dan sistem",
            (Language::German, Text) => "Text und System",
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
            Language::English => "Tests the current draft and library matching only — not desktop integration. It does not test capture, focus, portals, sensitive fields, or insertion; command snippets are not run. Expansions replace only their trigger and keep surrounding preview text. Clear the preview to start over.",
            Language::Indonesian => "Hanya menguji pencocokan draf saat ini dan pustaka — bukan integrasi desktop. Tidak menguji tangkapan, fokus, portal, kolom sensitif, atau penyisipan; cuplikan perintah tidak dijalankan. Ekspansi hanya mengganti pemicunya dan mempertahankan teks pratinjau di sekitarnya. Hapus pratinjau untuk memulai lagi.",
            Language::German => "Testet nur das Matching des aktuellen Entwurfs und der Bibliothek — keine Desktop-Integration. Erfassung, Fokus, Portale, sensible Felder und Einfügen werden nicht getestet; Befehls-Snippets laufen nicht. Erweiterungen ersetzen nur ihren Auslöser und behalten den umgebenden Vorschautext bei. Vorschau leeren, um neu zu beginnen.",
        }
    }

    pub fn try_live_short(&self) -> &'static str {
        match self.lang {
            Language::English => "Tests matching only, not desktop integration",
            Language::Indonesian => "Hanya menguji pencocokan, bukan integrasi desktop",
            Language::German => "Testet nur Matching, keine Desktop-Integration",
        }
    }

    pub fn try_live_cumulative_notice(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Each trigger replaces only itself. Earlier preview text stays; clear the preview before a separate test."
            }
            Language::Indonesian => {
                "Setiap pemicu hanya menggantikan dirinya. Teks pratinjau sebelumnya tetap ada; hapus pratinjau sebelum pengujian terpisah."
            }
            Language::German => {
                "Jeder Trigger ersetzt nur sich selbst. Vorheriger Vorschautext bleibt erhalten; für einen neuen Test die Vorschau leeren."
            }
        }
    }

    pub fn try_live_form_detected(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Form snippets can't be simulated here. Typed forms need a route that verifies the trigger at the cursor; key-synthesis routes such as evdev + libei cannot do that. Use a verified input-method route, or remove the `{{field:...}}` marker."
            }
            Language::Indonesian => {
                "Cuplikan formulir tidak dapat disimulasikan di sini. Formulir ketik memerlukan rute yang memverifikasi pemicu di kursor; rute sintesis tombol seperti evdev + libei tidak dapat melakukannya. Gunakan rute input-method terverifikasi atau hapus penanda `{{field:...}}`."
            }
            Language::German => {
                "Formular-Snippets lassen sich hier nicht simulieren. Getippte Formulare benötigen eine Route, die den Trigger am Cursor verifiziert; Tasten-Syntheserouten wie evdev + libei können das nicht. Verwenden Sie eine verifizierte Input-Method-Route oder entfernen Sie den Marker `{{field:...}}`."
            }
        }
    }

    pub fn try_live_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Type a trigger; surrounding text stays…",
            Language::Indonesian => "Ketik pemicu; teks di sekitarnya tetap ada…",
            Language::German => "Auslöser tippen; umgebender Text bleibt…",
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

    pub fn try_live_draft(&self) -> &'static str {
        match self.lang {
            Language::English => "Testing the current unsaved draft",
            Language::Indonesian => "Menguji draf saat ini yang belum disimpan",
            Language::German => "Der aktuelle ungespeicherte Entwurf wird getestet",
        }
    }

    pub fn clear(&self) -> &'static str {
        match self.lang {
            Language::English => "Clear preview",
            Language::Indonesian => "Hapus pratinjau",
            Language::German => "Vorschau leeren",
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
}
