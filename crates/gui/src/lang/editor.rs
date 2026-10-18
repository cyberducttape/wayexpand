use super::{Language, Strings};

impl Strings {
    pub fn snippet_details(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet details",
            Language::Indonesian => "Detail cuplikan",
            Language::German => "Snippet-Details",
        }
    }

    pub fn trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Trigger",
            Language::Indonesian => "Pemicu",
            Language::German => "Auslöser",
        }
    }

    pub fn trigger_hint(&self) -> &'static str {
        match self.lang {
            Language::English => ";;hello",
            Language::Indonesian => ";;hello",
            Language::German => ";;hallo",
        }
    }

    pub fn duplicate_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: another snippet already uses this trigger; saving will be rejected.",
            Language::Indonesian => "Peringatan: cuplikan lain sudah menggunakan pemicu ini; penyimpanan akan ditolak.",
            Language::German => "Warnung: Ein anderes Snippet verwendet bereits diesen Auslöser; das Speichern wird abgelehnt.",
        }
    }

    pub fn trigger_tip(&self) -> &'static str {
        match self.lang {
            Language::English => "Tip: use a distinctive prefix such as ;; or : to avoid accidental matches.",
            Language::Indonesian => "Tips: gunakan awalan khas seperti ;; atau : untuk menghindari kecocokan yang tidak disengaja.",
            Language::German => "Tipp: Verwenden Sie ein eindeutiges Präfix wie ;; oder :, um versehentliche Treffer zu vermeiden.",
        }
    }

    pub fn description(&self) -> &'static str {
        match self.lang {
            Language::English => "Description",
            Language::Indonesian => "Deskripsi",
            Language::German => "Beschreibung",
        }
    }

    pub fn tags(&self) -> &'static str {
        match self.lang {
            Language::English => "Tags",
            Language::Indonesian => "Tag",
            Language::German => "Tags",
        }
    }

    pub fn category(&self) -> &'static str {
        match self.lang {
            Language::English => "Category",
            Language::Indonesian => "Kategori",
            Language::German => "Kategorie",
        }
    }

    pub fn category_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "productivity, shortcuts, custom",
            Language::Indonesian => "produktivitas, pintasan, kustom",
            Language::German => "Produktivität, Verknüpfungen, Benutzerdefiniert",
        }
    }

    pub fn output_file(&self) -> &'static str {
        match self.lang {
            Language::English => "Write to file",
            Language::Indonesian => "Tulis ke berkas",
            Language::German => "In Datei schreiben",
        }
    }

    pub fn output_file_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "Dockerfile, site.yml (optional)",
            Language::Indonesian => "Dockerfile, site.yml (opsional)",
            Language::German => "Dockerfile, site.yml (optional)",
        }
    }

    pub fn output_file_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "`wayexpand new` fills in this snippet's fields and writes it to this file in the current directory"
            }
            Language::Indonesian => {
                "`wayexpand new` mengisi kolom cuplikan ini dan menulisnya ke berkas ini di direktori saat ini"
            }
            Language::German => {
                "`wayexpand new` füllt die Felder dieses Snippets aus und schreibt es in diese Datei im aktuellen Verzeichnis"
            }
        }
    }

    pub fn existing(&self) -> &'static str {
        match self.lang {
            Language::English => "Choose…",
            Language::Indonesian => "Pilih…",
            Language::German => "Auswählen…",
        }
    }

    pub fn app_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "Only in these apps",
            Language::Indonesian => "Hanya di aplikasi ini",
            Language::German => "Nur in diesen Apps",
        }
    }

    pub fn detect_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Use current app",
            Language::Indonesian => "Gunakan aplikasi saat ini",
            Language::German => "Aktuelle App verwenden",
        }
    }

    pub fn detect_app_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Click this, then click a window of the app this snippet is for within 15 seconds (KDE Plasma only for now)",
            Language::Indonesian => "Klik ini, lalu klik jendela aplikasi untuk cuplikan ini dalam 15 detik (saat ini hanya KDE Plasma)",
            Language::German => "Hier klicken und dann innerhalb von 15 Sekunden ein Fenster der App anklicken, für die das Snippet gilt (nur KDE Plasma)",
        }
    }

    pub fn window_tracking_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: if window tracking isn't available on your compositor, this snippet will never match rather than matching everywhere.",
            Language::Indonesian => "Peringatan: jika pelacakan jendela tidak tersedia di compositor Anda, cuplikan ini tidak akan pernah cocok alih-alih cocok di mana saja.",
            Language::German => "Warnung: Wenn die Fenster-Verfolgung in Ihrem Kompositor nicht verfügbar ist, passt dieses Snippet nie, anstatt überall zu passen.",
        }
    }

    pub fn enabled(&self) -> &'static str {
        match self.lang {
            Language::English => "Enabled",
            Language::Indonesian => "Aktif",
            Language::German => "Aktiviert",
        }
    }

    pub fn immediate(&self) -> &'static str {
        match self.lang {
            Language::English => "Immediate",
            Language::Indonesian => "Langsung",
            Language::German => "Sofort",
        }
    }

    pub fn word_boundary(&self) -> &'static str {
        match self.lang {
            Language::English => "Word boundary",
            Language::Indonesian => "Batas kata",
            Language::German => "Wortgrenze",
        }
    }

    pub fn propagate_case(&self) -> &'static str {
        match self.lang {
            Language::English => "Follow trigger capitalization",
            Language::Indonesian => "Ikuti kapitalisasi pemicu",
            Language::German => "Groß-/Kleinschreibung anpassen",
        }
    }

    pub fn propagate_case_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Typing the trigger in UPPERCASE or Capitalized form applies the same casing to the replacement"
            }
            Language::Indonesian => {
                "Mengetik pemicu dalam HURUF BESAR atau Berawalan Kapital menerapkan kapitalisasi yang sama pada teks pengganti"
            }
            Language::German => {
                "Wird der Auslöser in GROSSBUCHSTABEN oder Großschreibung eingegeben, erhält der Ersatztext dieselbe Schreibweise"
            }
        }
    }

    pub fn replacement(&self) -> &'static str {
        match self.lang {
            Language::English => "Replacement",
            Language::Indonesian => "Teks pengganti",
            Language::German => "Ersatz",
        }
    }

    pub fn insertion_limit_warning(&self, count: usize, limit: usize, mode: &str) -> String {
        match self.lang {
            Language::English => format!(
                "This snippet has {count} characters, but the active {mode} mode supports at most {limit}. It may be refused before insertion."
            ),
            Language::Indonesian => format!(
                "Cuplikan ini memiliki {count} karakter, tetapi mode {mode} yang aktif mendukung paling banyak {limit}. Cuplikan ini mungkin ditolak sebelum disisipkan."
            ),
            Language::German => format!(
                "Dieses Snippet hat {count} Zeichen, aber der aktive Modus {mode} unterstützt höchstens {limit}. Die Einfügung wird möglicherweise abgelehnt."
            ),
        }
    }

    pub fn save_changes(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes",
            Language::Indonesian => "Simpan perubahan",
            Language::German => "Änderungen speichern",
        }
    }

    pub fn save_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Save changes (Ctrl+S)",
            Language::Indonesian => "Simpan perubahan (Ctrl+S)",
            Language::German => "Änderungen speichern (Strg+S)",
        }
    }

    pub fn delete(&self) -> &'static str {
        match self.lang {
            Language::English => "Delete…",
            Language::Indonesian => "Hapus…",
            Language::German => "Löschen…",
        }
    }

    pub fn preview(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview",
            Language::Indonesian => "Pratinjau",
            Language::German => "Vorschau",
        }
    }

    pub fn input(&self) -> &'static str {
        match self.lang {
            Language::English => "Input",
            Language::Indonesian => "Masukan",
            Language::German => "Eingabe",
        }
    }

    pub fn input_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "text containing the trigger",
            Language::Indonesian => "teks yang berisi pemicu",
            Language::German => "Text mit dem Auslöser",
        }
    }

    pub fn use_trigger(&self) -> &'static str {
        match self.lang {
            Language::English => "Use trigger",
            Language::Indonesian => "Gunakan pemicu",
            Language::German => "Auslöser verwenden",
        }
    }

    pub fn copy(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy",
            Language::Indonesian => "Salin",
            Language::German => "Kopieren",
        }
    }

    pub fn copy_tooltip(&self) -> &'static str {
        match self.lang {
            Language::English => "Copy the previewed output",
            Language::Indonesian => "Salin keluaran pratinjau",
            Language::German => "Kopieren Sie die vorbereitete Ausgabe",
        }
    }

    pub fn template_variables(&self) -> &'static str {
        match self.lang {
            Language::English => "Template variables",
            Language::Indonesian => "Variabel templat",
            Language::German => "Template-Variablen",
        }
    }

    pub fn template_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Insert a safe built-in value into the replacement.",
            Language::Indonesian => "Sisipkan nilai bawaan yang aman ke dalam teks pengganti.",
            Language::German => "Fügen Sie einen sicheren integrierten Wert in den Ersatz ein.",
        }
    }

    pub fn dynamic_command(&self) -> &'static str {
        match self.lang {
            Language::English => "Dynamic command (optional)",
            Language::Indonesian => "Perintah dinamis (opsional)",
            Language::German => "Dynamischer Befehl (optional)",
        }
    }

    pub fn execution_type(&self) -> &'static str {
        match self.lang {
            Language::English => "Execution type",
            Language::Indonesian => "Jenis eksekusi",
            Language::German => "Ausführungsart",
        }
    }

    pub fn direct_executable(&self) -> &'static str {
        match self.lang {
            Language::English => "Direct executable",
            Language::Indonesian => "Program langsung",
            Language::German => "Direktes Programm",
        }
    }

    pub fn managed_action(&self) -> &'static str {
        match self.lang {
            Language::English => "Managed action",
            Language::Indonesian => "Aksi terkelola",
            Language::German => "Verwaltete Aktion",
        }
    }

    pub fn action(&self) -> &'static str {
        match self.lang {
            Language::English => "Action",
            Language::Indonesian => "Aksi",
            Language::German => "Aktion",
        }
    }

    pub fn select_or_type_action(&self) -> &'static str {
        match self.lang {
            Language::English => "Select or type an action",
            Language::Indonesian => "Pilih atau ketik aksi",
            Language::German => "Aktion auswählen oder eingeben",
        }
    }

    pub fn action_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "cluster-status",
            Language::Indonesian => "cluster-status",
            Language::German => "cluster-status",
        }
    }

    pub fn command_checkbox(&self) -> &'static str {
        match self.lang {
            Language::English => "Run a direct program when this snippet matches",
            Language::Indonesian => "Jalankan program langsung saat cuplikan ini cocok",
            Language::German => "Führen Sie ein direktes Programm aus, wenn dieses Snippet passt",
        }
    }

    pub fn command_help(&self) -> &'static str {
        match self.lang {
            Language::English => "Only the configured executable is run; shell syntax is never interpreted. Arguments are entered one per line.",
            Language::Indonesian => "Hanya program yang dikonfigurasi yang dijalankan; sintaks shell tidak pernah ditafsirkan. Masukkan satu argumen per baris.",
            Language::German => "Nur die konfigurierte ausführbare Datei wird ausgeführt; Shell-Syntax wird nie interpretiert. Argumente werden eine pro Zeile eingegeben.",
        }
    }

    pub fn command_warning(&self) -> &'static str {
        match self.lang {
            Language::English => "Warning: this runs a local executable when the trigger matches.",
            Language::Indonesian => {
                "Peringatan: ini menjalankan program lokal saat pemicunya cocok."
            }
            Language::German => {
                "Warnung: Dieses Programm wird ausgeführt, wenn der Auslöser passt."
            }
        }
    }

    pub fn program(&self) -> &'static str {
        match self.lang {
            Language::English => "Program",
            Language::Indonesian => "Program",
            Language::German => "Programm",
        }
    }

    pub fn program_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "uname",
            Language::Indonesian => "uname",
            Language::German => "uname",
        }
    }

    pub fn timeout_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Timeout ms",
            Language::Indonesian => "Batas waktu (ms)",
            Language::German => "Zeitüberschreitung ms",
        }
    }

    pub fn cache_ms(&self) -> &'static str {
        match self.lang {
            Language::English => "Cache ms",
            Language::Indonesian => "Cache (ms)",
            Language::German => "Cache ms",
        }
    }

    pub fn arguments(&self) -> &'static str {
        match self.lang {
            Language::English => "Arguments",
            Language::Indonesian => "Argumen",
            Language::German => "Argumente",
        }
    }

    pub fn command_backed_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "The typed text comes from the command below. This replacement is kept but not used while the command is enabled, including when the command fails."
            }
            Language::Indonesian => {
                "Teks yang diketik berasal dari perintah di bawah. Teks pengganti ini disimpan tetapi tidak digunakan selama perintah aktif, termasuk saat perintah gagal."
            }
            Language::German => {
                "Der eingefügte Text stammt vom Befehl unten. Dieser Ersatztext bleibt erhalten, wird aber nicht verwendet, solange der Befehl aktiv ist – auch nicht, wenn der Befehl fehlschlägt."
            }
        }
    }
}
