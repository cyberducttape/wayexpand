use super::{Language, Strings};

impl Strings {
    pub fn pack_rollback_complete(&self) -> &'static str {
        match self.lang {
            Language::English => "Previous pack version restored and daemon reload requested",
            Language::Indonesian => {
                "Versi paket sebelumnya dipulihkan dan pemuatan ulang daemon diminta"
            }
            Language::German => {
                "Vorherige Paketversion wiederhergestellt und Neuladen des Daemons angefordert"
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

    pub fn pack_compatibility(
        &self,
        shells: &[String],
        platforms: &[String],
        tools: &[String],
    ) -> String {
        let shells = shells.join(", ");
        let platforms = platforms.join(", ");
        let tools = if tools.is_empty() {
            "none".to_owned()
        } else {
            tools.join(", ")
        };
        match self.lang {
            Language::English => {
                format!("Compatibility: shells {shells}; platforms {platforms}; tools {tools}")
            }
            Language::Indonesian => {
                format!("Kompatibilitas: shell {shells}; platform {platforms}; alat {tools}")
            }
            Language::German => format!(
                "Kompatibilität: Shells {shells}; Plattformen {platforms}; Werkzeuge {tools}"
            ),
        }
    }

    pub fn pack_signature_unsigned_detail(&self) -> &'static str {
        match self.lang {
            Language::English => "No cryptographic signer identity is available for this pack.",
            Language::Indonesian => {
                "Identitas penanda tangan kriptografis tidak tersedia untuk paket ini."
            }
            Language::German => {
                "Für dieses Paket ist keine kryptografische Signiereridentität verfügbar."
            }
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

    pub fn try_live_stats(&self, expansions: usize, saved: usize) -> String {
        match self.lang {
            Language::English => format!(
                "{expansions} local preview match{} · {saved} preview keystrokes saved",
                if expansions == 1 { "" } else { "s" }
            ),
            // Indonesian nouns do not inflect for plural.
            Language::Indonesian => {
                format!("{expansions} kecocokan pratinjau lokal · {saved} ketukan pratinjau dihemat")
            }
            Language::German => format!(
                "{expansions} lokale Vorschau-Treffer · {saved} Tastenanschläge in der Vorschau gespart"
            ),
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
                "Copy-only in this session · Enter copies · switch to your app and paste with Ctrl+V"
            }
            Language::Indonesian => {
                "Hanya salin pada sesi ini · Enter menyalin · pindah ke aplikasi lalu tempel dengan Ctrl+V"
            }
            Language::German => {
                "In dieser Sitzung nur kopieren · Enter kopiert · zur App wechseln und mit Strg+V einfügen"
            }
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
            Language::English => "Not inserted",
            Language::Indonesian => "Tidak disisipkan",
            Language::German => "Nicht eingefügt",
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
}

impl Strings {
    /// Why the Quick Picker cannot use a listed snippet, or `None` if it can.
    pub(crate) fn picker_unavailable_reason(
        &self,
        availability: crate::picker::Availability,
    ) -> Option<&'static str> {
        use crate::picker::Availability as A;
        Some(match (availability, self.lang) {
            (A::Available | A::Hidden | A::NeedsInput | A::RunsCommand, _) => return None,
            (A::AppRestricted, Language::English) => {
                "Limited to specific apps. Copying cannot check where it is pasted, so type its trigger in that app."
            }
            (A::AppRestricted, Language::German) => {
                "Auf bestimmte Apps beschränkt. Beim Kopieren lässt sich das Ziel nicht prüfen; tippen Sie den Auslöser in dieser App."
            }
            (A::AppRestricted, Language::Indonesian) => {
                "Terbatas untuk aplikasi tertentu. Menyalin tidak bisa memeriksa tujuan tempel, jadi ketik pemicunya di aplikasi itu."
            }
            (A::ReadsClipboard, Language::English) => {
                "Reads the clipboard, so it is not copied for pasting. Type its trigger instead."
            }
            (A::ReadsClipboard, Language::German) => {
                "Liest die Zwischenablage und wird deshalb nicht zum Einfügen kopiert. Tippen Sie stattdessen den Auslöser."
            }
            (A::ReadsClipboard, Language::Indonesian) => {
                "Membaca papan klip, jadi tidak disalin untuk ditempel. Ketik pemicunya saja."
            }
            (A::InvalidTemplate, Language::English) => {
                "This template is invalid and cannot be previewed safely."
            }
            (A::InvalidTemplate, Language::German) => {
                "Diese Vorlage ist ungültig und kann nicht sicher angezeigt werden."
            }
            (A::InvalidTemplate, Language::Indonesian) => {
                "Templat ini tidak valid dan tidak dapat dipratinjau dengan aman."
            }
            (A::PolicyRestricted, Language::English) => {
                "Organization policy does not allow copying this form output."
            }
            (A::PolicyRestricted, Language::German) => {
                "Die Organisationsrichtlinie erlaubt das Kopieren dieser Formulare nicht."
            }
            (A::PolicyRestricted, Language::Indonesian) => {
                "Kebijakan organisasi tidak mengizinkan penyalinan hasil formulir ini."
            }
        })
    }

    pub(crate) fn picker_entry_status(
        &self,
        availability: crate::picker::Availability,
        fields: usize,
        direct_insert: bool,
        file_template: bool,
    ) -> String {
        use crate::picker::Availability as A;
        match (availability, self.lang) {
            (A::Available, Language::English) if file_template => {
                "File template · Ready to copy".into()
            }
            (A::Available, Language::German) if file_template => {
                "Dateivorlage · Bereit zum Kopieren".into()
            }
            (A::Available, Language::Indonesian) if file_template => {
                "Templat berkas · Siap disalin".into()
            }
            (A::Available, Language::English) if direct_insert => {
                "Text snippet · Ready to insert".into()
            }
            (A::Available, Language::German) if direct_insert => {
                "Textbaustein · Bereit zum Einfügen".into()
            }
            (A::Available, Language::Indonesian) if direct_insert => {
                "Cuplikan teks · Siap disisipkan".into()
            }
            (A::Available, Language::English) => "Text snippet · Ready to copy".into(),
            (A::Available, Language::German) => "Textbaustein · Bereit zum Kopieren".into(),
            (A::Available, Language::Indonesian) => "Cuplikan teks · Siap disalin".into(),
            (A::NeedsInput, Language::English) if file_template => {
                format!("File template · Requires {fields} fields")
            }
            (A::NeedsInput, Language::German) if file_template => {
                format!("Dateivorlage · {fields} Felder erforderlich")
            }
            (A::NeedsInput, Language::Indonesian) if file_template => {
                format!("Templat berkas · Perlu {fields} isian")
            }
            (A::NeedsInput, Language::English) => format!("Template · Requires {fields} fields"),
            (A::NeedsInput, Language::German) => format!("Vorlage · {fields} Felder erforderlich"),
            (A::NeedsInput, Language::Indonesian) => format!("Templat · Perlu {fields} isian"),
            (A::RunsCommand, Language::English) => "Automation · Review required".into(),
            (A::RunsCommand, Language::German) => "Automatisierung · Prüfung erforderlich".into(),
            (A::RunsCommand, Language::Indonesian) => "Otomatisasi · Perlu ditinjau".into(),
            (A::AppRestricted, Language::English) => "Limited to specific apps".into(),
            (A::AppRestricted, Language::German) => "Auf bestimmte Apps beschränkt".into(),
            (A::AppRestricted, Language::Indonesian) => "Terbatas untuk aplikasi tertentu".into(),
            (A::ReadsClipboard, Language::English) => "Reads clipboard or secret data".into(),
            (A::ReadsClipboard, Language::German) => "Liest Zwischenablage oder Geheimnisse".into(),
            (A::ReadsClipboard, Language::Indonesian) => {
                "Membaca papan klip atau data rahasia".into()
            }
            (A::PolicyRestricted, Language::English) => "Blocked by organization policy".into(),
            (A::PolicyRestricted, Language::German) => {
                "Durch Organisationsrichtlinie blockiert".into()
            }
            (A::PolicyRestricted, Language::Indonesian) => "Diblokir kebijakan organisasi".into(),
            (A::InvalidTemplate, Language::English) => "Template unavailable".into(),
            (A::InvalidTemplate, Language::German) => "Vorlage nicht verfügbar".into(),
            (A::InvalidTemplate, Language::Indonesian) => "Templat tidak tersedia".into(),
            (A::Hidden, Language::English) => "Unavailable".into(),
            (A::Hidden, Language::German) => "Nicht verfügbar".into(),
            (A::Hidden, Language::Indonesian) => "Tidak tersedia".into(),
        }
    }

    pub(crate) fn picker_review_no_execution(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This is a review only. The Quick Picker will not execute this action."
            }
            Language::German => {
                "Dies ist nur eine Prüfung. Der Schnellwähler führt diese Aktion nicht aus."
            }
            Language::Indonesian => {
                "Ini hanya peninjauan. Quick Picker tidak akan menjalankan tindakan ini."
            }
        }
    }
}
