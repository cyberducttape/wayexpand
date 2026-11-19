use super::{Language, Strings};

impl Strings {
    pub fn generate_only_hint(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "Typing the trigger does nothing; the snippet only fills in the file above."
            }
            Language::Indonesian => {
                "Mengetik pemicu tidak melakukan apa pun; cuplikan hanya mengisi berkas di atas."
            }
            Language::German => {
                "Das Kürzel wird beim Tippen nicht ersetzt; das Snippet füllt nur die Datei oben."
            }
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

    pub fn reusable_snippet_tooltip(&self, trigger: &str) -> String {
        match self.lang {
            Language::English => format!("Insert the static text from {trigger}"),
            Language::Indonesian => format!("Sisipkan teks statis dari {trigger}"),
            Language::German => format!("Statischen Text aus {trigger} einfügen"),
        }
    }

    pub fn reusable_snippet_description(&self, trigger: &str, description: &str) -> String {
        match self.lang {
            Language::English => format!("{trigger}: {description}"),
            Language::Indonesian => format!("{trigger}: {description}"),
            Language::German => format!("{trigger}: {description}"),
        }
    }

    pub fn broker_action_unlisted(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This action is not listed in the broker policy visible to this GUI."
            }
            Language::Indonesian => {
                "Aksi ini tidak tercantum dalam kebijakan broker yang terlihat oleh GUI."
            }
            Language::German => {
                "Diese Aktion ist nicht in der für die GUI sichtbaren Broker-Richtlinie aufgeführt."
            }
        }
    }

    pub fn broker_schema_match(&self) -> &'static str {
        match self.lang {
            Language::English => "Form fields match the broker schema by name.",
            Language::Indonesian => "Nama kolom formulir cocok dengan skema broker.",
            Language::German => {
                "Die Formularfelder stimmen anhand ihrer Namen mit dem Broker-Schema überein."
            }
        }
    }

    pub fn broker_schema_missing(&self, names: &str) -> String {
        match self.lang {
            Language::English => format!("Add form fields for: {names}"),
            Language::Indonesian => format!("Tambahkan kolom formulir untuk: {names}"),
            Language::German => format!("Formularfelder hinzufügen für: {names}"),
        }
    }

    pub fn broker_schema_unexpected(&self, names: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Remove or rename fields not accepted by this action: {names}")
            }
            Language::Indonesian => {
                format!("Hapus atau ganti nama kolom yang tidak diterima aksi ini: {names}")
            }
            Language::German => {
                format!("Nicht akzeptierte Felder entfernen oder umbenennen: {names}")
            }
        }
    }

    pub fn broker_schema_constraint_mismatch(&self, names: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Form input may exceed the broker constraints for: {names}")
            }
            Language::Indonesian => {
                format!("Input formulir mungkin melampaui batasan broker untuk: {names}")
            }
            Language::German => format!(
                "Formulareingaben können die Broker-Beschränkungen überschreiten für: {names}"
            ),
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
