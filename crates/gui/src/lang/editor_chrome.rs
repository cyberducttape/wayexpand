use super::{Language, Strings};

impl Strings {
    pub fn preview_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Preview app",
            Language::Indonesian => "Aplikasi pratinjau",
            Language::German => "Vorschau-App",
        }
    }

    pub fn preview_app_hint(&self) -> &'static str {
        match self.lang {
            Language::English => "leave empty for no focused app",
            Language::Indonesian => "kosongkan jika tidak ada aplikasi yang fokus",
            Language::German => "leer lassen für keine fokussierte App",
        }
    }

    pub fn run_once(&self) -> &'static str {
        match self.lang {
            Language::English => "Run locally (advanced)",
            Language::Indonesian => "Jalankan secara lokal (lanjutan)",
            Language::German => "Lokal ausführen (erweitert)",
        }
    }

    /// The preview runs in a transient unit with the daemon's sandbox.
    pub fn run_in_service_sandbox(&self) -> &'static str {
        match self.lang {
            Language::English => "Run in service sandbox",
            Language::Indonesian => "Jalankan di sandbox layanan",
            Language::German => "In der Dienst-Sandbox ausführen",
        }
    }

    pub fn run_through_broker(&self) -> &'static str {
        match self.lang {
            Language::English => "Run through broker",
            Language::Indonesian => "Jalankan melalui broker",
            Language::German => "Über Broker ausführen",
        }
    }

    pub fn running(&self) -> &'static str {
        match self.lang {
            Language::English => "Running…",
            Language::Indonesian => "Sedang berjalan…",
            Language::German => "Läuft…",
        }
    }

    pub fn managed_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This named action runs through the configured Action Broker and is subject to its allowlist and execution policy. Broker auditing is optional and health-dependent; check wayexpand doctor --json for audit health."
            }
            Language::Indonesian => {
                "Aksi bernama ini berjalan melalui Action Broker yang dikonfigurasi dan tunduk pada daftar izin serta kebijakan eksekusinya. Audit broker bersifat opsional dan bergantung pada kesehatannya; periksa wayexpand doctor --json untuk kesehatan audit."
            }
            Language::German => {
                "Diese benannte Aktion läuft über den konfigurierten Action Broker und unterliegt dessen Zulassungsliste und Ausführungsrichtlinien. Das Broker-Audit ist optional und vom Zustand des Protokolls abhängig; den Audit-Status zeigt wayexpand doctor --json."
            }
        }
    }

    pub fn direct_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This direct program runs locally with desktop-user privileges, outside the daemon and broker sandboxes. It is not recorded in the Action Broker audit; use only for trusted commands."
            }
            Language::Indonesian => {
                "Program langsung ini berjalan secara lokal dengan hak pengguna desktop, di luar sandbox daemon dan broker. Program ini tidak dicatat dalam audit Action Broker; gunakan hanya untuk perintah tepercaya."
            }
            Language::German => {
                "Dieses Programm läuft lokal mit den Rechten des Desktop-Benutzers außerhalb der Daemon- und Broker-Sandbox. Es wird nicht im Action-Broker-Audit protokolliert; nur für vertrauenswürdige Befehle verwenden."
            }
        }
    }

    pub fn sandboxed_direct_command_preview_help(&self) -> &'static str {
        match self.lang {
            Language::English => {
                "This direct program runs as your desktop user in a transient systemd unit with WayExpand's service sandbox restrictions. It is not recorded in the Action Broker audit; use only for trusted commands."
            }
            Language::Indonesian => {
                "Program langsung ini berjalan sebagai pengguna desktop Anda di unit systemd sementara dengan pembatasan sandbox layanan WayExpand. Program ini tidak dicatat dalam audit Action Broker; gunakan hanya untuk perintah tepercaya."
            }
            Language::German => {
                "Dieses Programm läuft als Ihr Desktop-Benutzer in einer temporären systemd-Unit mit den Sandbox-Beschränkungen des WayExpand-Dienstes. Es wird nicht im Action-Broker-Audit protokolliert; nur für vertrauenswürdige Befehle verwenden."
            }
        }
    }

    pub fn detecting_app(&self) -> &'static str {
        match self.lang {
            Language::English => "Now click a window of the app…",
            Language::Indonesian => "Sekarang klik jendela aplikasinya…",
            Language::German => "Jetzt ein Fenster der App anklicken…",
        }
    }

    pub fn stopping_app_detection(&self) -> &'static str {
        match self.lang {
            Language::English => "Stopping application detection…",
            Language::Indonesian => "Menghentikan deteksi aplikasi…",
            Language::German => "Anwendungserkennung wird beendet…",
        }
    }

    pub fn no_description(&self) -> &'static str {
        match self.lang {
            Language::English => "No description",
            Language::Indonesian => "Tanpa deskripsi",
            Language::German => "Keine Beschreibung",
        }
    }

    pub fn click_to_enable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to enable",
            Language::Indonesian => "Klik untuk mengaktifkan",
            Language::German => "Zum Aktivieren klicken",
        }
    }

    pub fn click_to_disable(&self) -> &'static str {
        match self.lang {
            Language::English => "Click to disable",
            Language::Indonesian => "Klik untuk menonaktifkan",
            Language::German => "Zum Deaktivieren klicken",
        }
    }

    pub fn selection_stale(&self) -> &'static str {
        match self.lang {
            Language::English => "Selection is out of date; choose a snippet again.",
            Language::Indonesian => "Pilihan sudah usang; pilih cuplikan lagi.",
            Language::German => "Auswahl ist veraltet; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn draft_unavailable(&self) -> &'static str {
        match self.lang {
            Language::English => "Snippet draft unavailable; choose a snippet again.",
            Language::Indonesian => "Draf cuplikan tidak tersedia; pilih cuplikan lagi.",
            Language::German => "Snippet-Entwurf nicht verfügbar; bitte erneut ein Snippet wählen.",
        }
    }

    pub fn no_snippets_match_filter(&self) -> &'static str {
        match self.lang {
            Language::English => "No snippets match this filter.",
            Language::Indonesian => "Tidak ada cuplikan yang cocok dengan filter ini.",
            Language::German => "Keine Snippets entsprechen diesem Filter.",
        }
    }

    pub fn configuration_file(&self) -> &'static str {
        match self.lang {
            Language::English => "Configuration",
            Language::Indonesian => "Konfigurasi",
            Language::German => "Konfiguration",
        }
    }
}

#[cfg(test)]
mod preview_help_tests {
    use super::*;

    #[test]
    fn direct_preview_help_matches_the_selected_execution_boundary() {
        for language in [Language::English, Language::Indonesian, Language::German] {
            let strings = Strings::new(language);
            let local = strings.direct_command_preview_help();
            let sandboxed = strings.sandboxed_direct_command_preview_help();

            assert!(!local.is_empty());
            assert!(sandboxed.contains("systemd"));
            assert_ne!(local, sandboxed);
        }
    }
}
