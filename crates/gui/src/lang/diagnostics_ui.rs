//! Diagnostics window text.

impl super::Strings {
    pub(crate) fn dg_report_likely_cause(&self) -> &'static str {
        match self.lang {
            super::Language::English => "Likely cause",
            super::Language::German => "Wahrscheinliche Ursache",
            super::Language::Indonesian => "Kemungkinan penyebab",
        }
    }

    pub(crate) fn dg_report_next_action(&self) -> &'static str {
        match self.lang {
            super::Language::English => "Next action",
            super::Language::German => "Nächster Schritt",
            super::Language::Indonesian => "Tindakan berikutnya",
        }
    }

    pub(crate) fn dg_installation_identity_issue(&self, issue: &str) -> &'static str {
        match (self.lang, issue) {
            (super::Language::English, "path") => {
                "PATH starts a different WayExpand version than this application."
            }
            (super::Language::German, "path") => {
                "PATH startet eine andere WayExpand-Version als diese Anwendung."
            }
            (super::Language::Indonesian, "path") => {
                "PATH menjalankan versi WayExpand yang berbeda dari aplikasi ini."
            }
            (super::Language::English, _) => {
                "The running WayExpand service belongs to a different installation."
            }
            (super::Language::German, _) => {
                "Der laufende WayExpand-Dienst gehört zu einer anderen Installation."
            }
            (super::Language::Indonesian, _) => {
                "Layanan WayExpand yang berjalan berasal dari instalasi yang berbeda."
            }
        }
    }

    pub(crate) fn dg_installation_component(&self, component: &str) -> &'static str {
        let key = match component {
            "xdg_config_home" => "configuration_location",
            "xdg_data_home" => "application_data_location",
            "cli" => "command_line_app",
            "gui" => "desktop_app",
            "daemon" => "background_service",
            "desktop" => "application_launcher",
            "ibus" => "ibus_integration",
            "ibus_engine_executable" => "ibus_engine",
            "installer_contract" => "installation_manifest",
            "icon" => "application_icon",
            "default_config_template" => "starter_configuration",
            "control_service" => "control_service",
            "input_method_service" => "input_service",
            "evdev_service" => "optional_input_service",
            "active_configuration" => "active_configuration",
            _ => "unknown_component",
        };
        match (self.lang, key) {
            (super::Language::English, "configuration_location") => "Configuration location",
            (super::Language::German, "configuration_location") => "Konfigurationsordner",
            (super::Language::Indonesian, "configuration_location") => "Lokasi konfigurasi",
            (super::Language::English, "application_data_location") => "Application data folder",
            (super::Language::German, "application_data_location") => "Anwendungsdatenordner",
            (super::Language::Indonesian, "application_data_location") => "Folder data aplikasi",
            (super::Language::English, "command_line_app") => "WayExpand command",
            (super::Language::German, "command_line_app") => "WayExpand-Befehl",
            (super::Language::Indonesian, "command_line_app") => "Perintah WayExpand",
            (super::Language::English, "desktop_app") => "WayExpand application",
            (super::Language::German, "desktop_app") => "WayExpand-Anwendung",
            (super::Language::Indonesian, "desktop_app") => "Aplikasi WayExpand",
            (super::Language::English, "background_service") => "Background service",
            (super::Language::German, "background_service") => "Hintergrunddienst",
            (super::Language::Indonesian, "background_service") => "Layanan latar belakang",
            (super::Language::English, "application_launcher") => "Application menu entry",
            (super::Language::German, "application_launcher") => "Eintrag im Anwendungsmenü",
            (super::Language::Indonesian, "application_launcher") => "Entri menu aplikasi",
            (super::Language::English, "ibus_integration") => "IBus integration",
            (super::Language::German, "ibus_integration") => "IBus-Integration",
            (super::Language::Indonesian, "ibus_integration") => "Integrasi IBus",
            (super::Language::English, "ibus_engine") => "IBus engine",
            (super::Language::German, "ibus_engine") => "IBus-Engine",
            (super::Language::Indonesian, "ibus_engine") => "Mesin IBus",
            (super::Language::English, "installation_manifest") => "Installation manifest",
            (super::Language::German, "installation_manifest") => "Installationsmanifest",
            (super::Language::Indonesian, "installation_manifest") => "Manifest instalasi",
            (super::Language::English, "application_icon") => "Application icon",
            (super::Language::German, "application_icon") => "Anwendungssymbol",
            (super::Language::Indonesian, "application_icon") => "Ikon aplikasi",
            (super::Language::English, "starter_configuration") => "Starter configuration",
            (super::Language::German, "starter_configuration") => "Beispielkonfiguration",
            (super::Language::Indonesian, "starter_configuration") => "Konfigurasi awal",
            (super::Language::English, "control_service") => "Control service",
            (super::Language::German, "control_service") => "Steuerungsdienst",
            (super::Language::Indonesian, "control_service") => "Layanan kontrol",
            (super::Language::English, "input_service") => "Input service",
            (super::Language::German, "input_service") => "Eingabedienst",
            (super::Language::Indonesian, "input_service") => "Layanan input",
            (super::Language::English, "optional_input_service") => "Optional input service",
            (super::Language::German, "optional_input_service") => "Optionaler Eingabedienst",
            (super::Language::Indonesian, "optional_input_service") => "Layanan input opsional",
            (super::Language::English, "active_configuration") => "Your snippet configuration",
            (super::Language::German, "active_configuration") => "Ihre Snippet-Konfiguration",
            (super::Language::Indonesian, "active_configuration") => "Konfigurasi snippet Anda",
            (super::Language::English, _) => "Another installation component",
            (super::Language::German, _) => "Eine weitere Installationskomponente",
            (super::Language::Indonesian, _) => "Komponen instalasi lain",
        }
    }
}
