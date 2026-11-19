use super::{Language, Strings};

impl Strings {
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
