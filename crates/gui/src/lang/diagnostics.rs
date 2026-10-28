use super::{Language, Strings};
use crate::*;

impl Strings {
    pub fn runtime_dashboard(&self) -> &'static str {
        match self.lang {
            Language::English => "Active session dashboard",
            Language::Indonesian => "Dasbor sesi aktif",
            Language::German => "Aktive Sitzungsübersicht",
        }
    }

    pub fn route_assessment_title(&self) -> &'static str {
        match self.lang {
            Language::English => "What this route can do",
            Language::Indonesian => "Kemampuan rute ini",
            Language::German => "Was diese Route kann",
        }
    }

    pub fn capture_availability_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Capture availability",
            Language::Indonesian => "Ketersediaan tangkapan",
            Language::German => "Verfügbarkeit der Erfassung",
        }
    }

    pub fn insertion_safety_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Insertion safety",
            Language::Indonesian => "Keamanan penyisipan",
            Language::German => "Sicherheit der Ausgabe",
        }
    }

    pub fn insertion_assurance_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "The daemon has not reported replacement guarantees.",
            Language::Indonesian => "Daemon belum melaporkan jaminan penggantian.",
            Language::German => "Der Daemon hat keine Ersetzungsgarantien gemeldet.",
        }
    }

    pub fn insertion_unicode_limited(&self) -> &'static str {
        match self.lang {
            Language::English => "Some Unicode text may not be supported by this route.",
            Language::Indonesian => "Beberapa teks Unicode mungkin tidak didukung rute ini.",
            Language::German => {
                "Einige Unicode-Zeichen werden von dieser Route möglicherweise nicht unterstützt."
            }
        }
    }

    pub fn insertion_unicode_unknown(&self) -> &'static str {
        match self.lang {
            Language::English => "Full Unicode support has not been reported for this route.",
            Language::Indonesian => "Dukungan Unicode penuh belum dilaporkan untuk rute ini.",
            Language::German => "Volle Unicode-Unterstützung wurde für diese Route nicht gemeldet.",
        }
    }

    pub fn compositor_permissions_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Compositor permissions",
            Language::Indonesian => "Izin kompositor",
            Language::German => "Compositor-Berechtigungen",
        }
    }

    pub fn focus_visibility_label(&self) -> &'static str {
        match self.lang {
            Language::English => "Focus visibility",
            Language::Indonesian => "Visibilitas fokus",
            Language::German => "Sichtbarkeit des Fokus",
        }
    }

    pub fn sensitive_verification_note(&self) -> &'static str {
        match self.lang {
            Language::English => "Sensitivity capability is a runtime report; certification requires compositor and real-client evidence.",
            Language::Indonesian => "Kemampuan deteksi bidang sensitif berasal dari laporan runtime; sertifikasi memerlukan bukti kompositor dan aplikasi nyata.",
            Language::German => "Die Erkennung sensibler Felder ist eine Laufzeitmeldung; eine Zertifizierung erfordert Nachweise mit Compositor und echten Anwendungen.",
        }
    }

    pub fn assessment_state(&self, state: crate::diagnostics::AssessmentState) -> &'static str {
        use crate::diagnostics::AssessmentState::*;
        match (self.lang, state) {
            (Language::English, Ready) => "Ready",
            (Language::Indonesian, Ready) => "Siap",
            (Language::German, Ready) => "Bereit",
            (Language::English, Warning) => "Limited",
            (Language::Indonesian, Warning) => "Terbatas",
            (Language::German, Warning) => "Eingeschränkt",
            (Language::English, Blocked) => "Blocked",
            (Language::Indonesian, Blocked) => "Diblokir",
            (Language::German, Blocked) => "Blockiert",
            (Language::English, Unknown) => "Unknown",
            (Language::Indonesian, Unknown) => "Tidak diketahui",
            (Language::German, Unknown) => "Unbekannt",
        }
    }

    pub fn capture_assessment_detail(
        &self,
        state: Option<crate::runtime::RouteState>,
        source: &str,
        exclusive: Option<bool>,
    ) -> String {
        match (self.lang, state, exclusive) {
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), Some(true)) => {
                format!("Active capture route {source} is connected and captures keys exclusively.")
            }
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), Some(false)) => {
                format!("Active capture route {source} is connected but non-exclusive; keys reach the app before WayExpand can react.")
            }
            (Language::English, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Active capture route {source} is connected; capture exclusivity is unreported.")
            }
            (
                Language::Indonesian,
                Some(wayexpand_core::DaemonRouteState::Connected),
                Some(true),
            ) => {
                format!(
                    "Rute tangkap aktif {source} terhubung dan menangkap tombol secara eksklusif."
                )
            }
            (
                Language::Indonesian,
                Some(wayexpand_core::DaemonRouteState::Connected),
                Some(false),
            ) => {
                format!("Rute tangkap aktif {source} terhubung tetapi tidak eksklusif; tombol mencapai aplikasi sebelum WayExpand dapat merespons.")
            }
            (Language::Indonesian, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Rute tangkap aktif {source} terhubung; status eksklusivitas tangkapan tidak dilaporkan.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), Some(true)) => {
                format!("Aktive Eingaberoute {source} ist verbunden und erfasst Tasten exklusiv.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), Some(false)) => {
                format!("Aktive Eingaberoute {source} ist verbunden, erfasst aber nicht exklusiv; Tasten erreichen die Anwendung, bevor WayExpand reagieren kann.")
            }
            (Language::German, Some(wayexpand_core::DaemonRouteState::Connected), None) => {
                format!("Aktive Eingaberoute {source} ist verbunden; die Exklusivität der Erfassung ist nicht bekannt.")
            }
            (_, Some(wayexpand_core::DaemonRouteState::PermissionRequired), _) => {
                self.permission_required_route_detail(source)
            }
            (_, Some(wayexpand_core::DaemonRouteState::PortalRevoked), _) => {
                self.portal_revoked_route_detail(source)
            }
            (
                Language::English,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Capture route {source} is not fully available yet.")
            }
            (
                Language::Indonesian,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Rute tangkap {source} belum sepenuhnya tersedia.")
            }
            (
                Language::German,
                Some(
                    wayexpand_core::DaemonRouteState::Reconnecting
                    | wayexpand_core::DaemonRouteState::Starting
                    | wayexpand_core::DaemonRouteState::Degraded,
                ),
                _,
            ) => {
                format!("Eingaberoute {source} ist noch nicht vollständig verfügbar.")
            }
            (
                Language::English,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Capture route {source} cannot currently run.")
            }
            (
                Language::Indonesian,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Rute tangkap {source} tidak dapat berjalan saat ini.")
            }
            (
                Language::German,
                Some(
                    wayexpand_core::DaemonRouteState::Unsupported
                    | wayexpand_core::DaemonRouteState::Failed
                    | wayexpand_core::DaemonRouteState::Stopped,
                ),
                _,
            ) => {
                format!("Eingaberoute {source} kann derzeit nicht ausgeführt werden.")
            }
            (Language::English, None, _) => {
                "The daemon has not reported an active capture route.".into()
            }
            (Language::Indonesian, None, _) => "Daemon belum melaporkan rute tangkap aktif.".into(),
            (Language::German, None, _) => {
                "Der Daemon hat keine aktive Eingaberoute gemeldet.".into()
            }
        }
    }

    fn permission_required_route_detail(&self, source: &str) -> String {
        match self.lang {
            Language::English => {
                format!("Capture route {source} is waiting for compositor or device permission.")
            }
            Language::Indonesian => {
                format!("Rute tangkap {source} menunggu izin kompositor atau perangkat.")
            }
            Language::German => {
                format!("Eingaberoute {source} wartet auf Compositor- oder Geräteberechtigung.")
            }
        }
    }

    fn portal_revoked_route_detail(&self, source: &str) -> String {
        match self.lang {
            Language::English => format!("Authorization for capture route {source} was revoked."),
            Language::Indonesian => format!("Otorisasi untuk rute tangkap {source} telah dicabut."),
            Language::German => {
                format!("Die Autorisierung für Eingaberoute {source} wurde widerrufen.")
            }
        }
    }

    pub fn focus_assessment_detail(
        &self,
        tracker: Option<bool>,
        exact_identity: Option<bool>,
    ) -> String {
        match (self.lang, tracker, exact_identity) {
            (Language::English, Some(true), Some(true)) => "Window tracker is connected and exposes exact toplevel identity.".into(),
            (Language::Indonesian, Some(true), Some(true)) => "Pelacak jendela terhubung dan memberikan identitas toplevel yang tepat.".into(),
            (Language::German, Some(true), Some(true)) => "Fenster-Tracker ist verbunden und liefert eine exakte Toplevel-Identität.".into(),
            (Language::English, Some(false), _) => "No window tracker is connected; application and focus changes are not visible.".into(),
            (Language::Indonesian, Some(false), _) => "Tidak ada pelacak jendela yang terhubung; perubahan aplikasi dan fokus tidak terlihat.".into(),
            (Language::German, Some(false), _) => "Kein Fenster-Tracker verbunden; Anwendungs- und Fokuswechsel sind nicht sichtbar.".into(),
            (Language::English, Some(true), Some(false)) => "Window changes are visible, but exact toplevel identity is unavailable.".into(),
            (Language::Indonesian, Some(true), Some(false)) => "Perubahan jendela terlihat, tetapi identitas toplevel yang tepat tidak tersedia.".into(),
            (Language::German, Some(true), Some(false)) => "Fensterwechsel sind sichtbar, aber die exakte Toplevel-Identität fehlt.".into(),
            (Language::English, _, _) => "Focus visibility has not been confirmed by the daemon.".into(),
            (Language::Indonesian, _, _) => "Visibilitas fokus belum dikonfirmasi oleh daemon.".into(),
            (Language::German, _, _) => "Die Fokus-Sichtbarkeit wurde vom Daemon nicht bestätigt.".into(),
        }
    }

    pub fn capture_route(&self) -> &'static str {
        match self.lang {
            Language::English => "Capture route",
            Language::Indonesian => "Rute tangkap",
            Language::German => "Eingaberoute",
        }
    }

    pub fn injection_backend(&self) -> &'static str {
        match self.lang {
            Language::English => "Injection backend",
            Language::Indonesian => "Backend penyisipan",
            Language::German => "Ausgabebackend",
        }
    }

    pub fn route_id(&self) -> &'static str {
        match self.lang {
            Language::English => "Route ID",
            Language::Indonesian => "ID rute",
            Language::German => "Routen-ID",
        }
    }

    pub fn compositor_version(&self) -> &'static str {
        match self.lang {
            Language::English => "Compositor",
            Language::Indonesian => "Kompositor",
            Language::German => "Compositor",
        }
    }

    pub fn permission_health(&self) -> &'static str {
        match self.lang {
            Language::English => "Permission health",
            Language::Indonesian => "Status izin",
            Language::German => "Berechtigungsstatus",
        }
    }

    pub fn average_latency(&self) -> &'static str {
        match self.lang {
            Language::English => "Average injection latency",
            Language::Indonesian => "Latensi penyisipan rata-rata",
            Language::German => "Durchschnittliche Ausgabelatenz",
        }
    }

    pub fn sensitive_verification(&self) -> &'static str {
        match self.lang {
            Language::English => "Sensitive-field protection",
            Language::Indonesian => "Perlindungan kolom sensitif",
            Language::German => "Schutz sensibler Felder",
        }
    }

    pub fn recent_expansion_outcomes(&self) -> &'static str {
        match self.lang {
            Language::English => "Recent expansion outcomes · content-free trace",
            Language::Indonesian => "Hasil ekspansi terbaru · jejak tanpa konten",
            Language::German => "Letzte Expansionsergebnisse · inhaltsfreie Spur",
        }
    }

    pub fn no_recent_outcomes(&self) -> &'static str {
        match self.lang {
            Language::English => "No output transactions recorded in this daemon session.",
            Language::Indonesian => {
                "Tidak ada transaksi keluaran yang tercatat di sesi daemon ini."
            }
            Language::German => {
                "In dieser Daemon-Sitzung wurden keine Ausgabetransaktionen erfasst."
            }
        }
    }

    pub fn trace_completion(&self, completion: &str) -> String {
        match (self.lang, completion) {
            (Language::English, "applied") => "Output transaction acknowledged".into(),
            (Language::Indonesian, "applied") => "Transaksi keluaran dikonfirmasi".into(),
            (Language::German, "applied") => "Ausgabetransaktion bestätigt".into(),
            (_, "inserted; cursor placement failed") => match self.lang {
                Language::English => "Text inserted; cursor placement failed".into(),
                Language::Indonesian => "Teks disisipkan; penempatan kursor gagal".into(),
                Language::German => "Text eingefügt; Cursorpositionierung fehlgeschlagen".into(),
            },
            (_, "not applied") => match self.lang {
                Language::English => "Output was not applied".into(),
                Language::Indonesian => "Keluaran tidak diterapkan".into(),
                Language::German => "Ausgabe wurde nicht angewendet".into(),
            },
            (_, "application outcome unknown") => match self.lang {
                Language::English => "Application outcome is unknown".into(),
                Language::Indonesian => "Hasil penerapan tidak diketahui".into(),
                Language::German => "Anwendungsergebnis ist unbekannt".into(),
            },
            (_, "partial result unknown") => match self.lang {
                Language::English => "Partial output; final result is unknown".into(),
                Language::Indonesian => "Keluaran sebagian; hasil akhir tidak diketahui".into(),
                Language::German => "Teilausgabe; Endergebnis unbekannt".into(),
            },
            (_, "blocked by organization policy") => match self.lang {
                Language::English => "Blocked by organization policy before output".into(),
                Language::Indonesian => "Diblokir kebijakan organisasi sebelum keluaran".into(),
                Language::German => {
                    "Vor der Ausgabe durch Organisationsrichtlinie blockiert".into()
                }
            },
            (_, "blocked by command policy") => match self.lang {
                Language::English => "Blocked by command policy before output".into(),
                Language::Indonesian => "Diblokir kebijakan perintah sebelum keluaran".into(),
                Language::German => "Vor der Ausgabe durch Befehlsrichtlinie blockiert".into(),
            },
            (_, "key release was not confirmed before output") => match self.lang {
                Language::English => "Blocked because key release could not be confirmed".into(),
                Language::Indonesian => {
                    "Diblokir karena pelepasan tombol tidak dapat dikonfirmasi".into()
                }
                Language::German => {
                    "Blockiert, weil das Loslassen der Taste nicht bestätigt wurde".into()
                }
            },
            (_, "input quiet check failed") => match self.lang {
                Language::English => "Blocked because input safety could not be checked".into(),
                Language::Indonesian => {
                    "Diblokir karena keamanan input tidak dapat diperiksa".into()
                }
                Language::German => {
                    "Blockiert, weil die Eingabesicherheit nicht geprüft werden konnte".into()
                }
            },
            (_, "input arrived before output") => match self.lang {
                Language::English => "Blocked because more input arrived before replacement".into(),
                Language::Indonesian => "Diblokir karena ada input baru sebelum penggantian".into(),
                Language::German => {
                    "Blockiert, weil vor dem Ersetzen weitere Eingabe eintraf".into()
                }
            },
            (_, "multiple inputs arrived before output") => match self.lang {
                Language::English => {
                    "Blocked because multiple inputs arrived before replacement".into()
                }
                Language::Indonesian => {
                    "Diblokir karena beberapa input tiba sebelum penggantian".into()
                }
                Language::German => {
                    "Blockiert, weil vor dem Ersetzen mehrere Eingaben eintrafen".into()
                }
            },
            (_, "command timed out before output") => match self.lang {
                Language::English => "Command timed out before output".into(),
                Language::Indonesian => "Perintah kehabisan waktu sebelum keluaran".into(),
                Language::German => "Zeitüberschreitung des Befehls vor der Ausgabe".into(),
            },
            (_, "action broker unavailable") => match self.lang {
                Language::English => "Action Broker was unavailable".into(),
                Language::Indonesian => "Action Broker tidak tersedia".into(),
                Language::German => "Action Broker war nicht verfügbar".into(),
            },
            (_, "command output exceeded policy limit") => match self.lang {
                Language::English => "Command output exceeded the policy limit".into(),
                Language::Indonesian => "Keluaran perintah melebihi batas kebijakan".into(),
                Language::German => "Befehlsausgabe überschritt das Richtlinienlimit".into(),
            },
            (_, "command blocked by policy") => match self.lang {
                Language::English => "Command was blocked by policy".into(),
                Language::Indonesian => "Perintah diblokir oleh kebijakan".into(),
                Language::German => "Befehl wurde durch Richtlinie blockiert".into(),
            },
            (_, "command context was no longer safe") => match self.lang {
                Language::English => {
                    "Command result was discarded because the context became unsafe".into()
                }
                Language::Indonesian => {
                    "Hasil perintah dibuang karena konteks tidak lagi aman".into()
                }
                Language::German => {
                    "Befehlsergebnis verworfen, weil der Kontext nicht mehr sicher war".into()
                }
            },
            (_, "command result became stale") => match self.lang {
                Language::English => "Command result was discarded after input changed".into(),
                Language::Indonesian => "Hasil perintah dibuang setelah input berubah".into(),
                Language::German => "Befehlsergebnis nach Eingabeänderung verworfen".into(),
            },
            (_, "command focus could not be verified") => match self.lang {
                Language::English => {
                    "Command result was discarded because focus was uncertain".into()
                }
                Language::Indonesian => {
                    "Hasil perintah dibuang karena fokus tidak dapat dipastikan".into()
                }
                Language::German => "Befehlsergebnis wegen unklarem Fokus verworfen".into(),
            },
            (_, "command worker queue unavailable") => match self.lang {
                Language::English => "Command workers were busy or unavailable".into(),
                Language::Indonesian => "Worker perintah sibuk atau tidak tersedia".into(),
                Language::German => "Befehlsworker waren ausgelastet oder nicht verfügbar".into(),
            },
            (_, "command execution failed before output") => match self.lang {
                Language::English => "Command execution failed before output".into(),
                Language::Indonesian => "Eksekusi perintah gagal sebelum keluaran".into(),
                Language::German => "Befehlsausführung vor der Ausgabe fehlgeschlagen".into(),
            },
            (_, "expansion could not be prepared") => match self.lang {
                Language::English => "Expansion could not be prepared".into(),
                Language::Indonesian => "Ekspansi tidak dapat disiapkan".into(),
                Language::German => "Expansion konnte nicht vorbereitet werden".into(),
            },
            (_, "form requires verified replacement") => match self.lang {
                Language::English => "This snippet asks for form fields, but the active route cannot verify the trigger at the cursor. Use an input-method route with verified replacement, or remove the `{{field:...}}` marker.".into(),
                Language::Indonesian => "Cuplikan ini meminta isian formulir, tetapi rute aktif tidak dapat memverifikasi pemicu di kursor. Gunakan rute input-method dengan penggantian terverifikasi, atau hapus penanda `{{field:...}}`.".into(),
                Language::German => "Dieses Snippet fragt Formularfelder ab, aber die aktive Route kann den Trigger am Cursor nicht verifizieren. Verwenden Sie eine Input-Method-Route mit verifizierter Ersetzung oder entfernen Sie den Marker `{{field:...}}`.".into(),
            },
            (_, "form requires exact input-target identity") => match self.lang {
                Language::English => "This form needs an exact identity for the original text input, which the active route does not provide.".into(),
                Language::Indonesian => "Formulir ini memerlukan identitas tepat untuk input teks asal, tetapi rute aktif tidak menyediakannya.".into(),
                Language::German => "Dieses Formular benötigt eine genaue Identität für das ursprüngliche Texteingabefeld, die die aktive Route nicht bereitstellt.".into(),
            },
            (_, "form values could not render the template") => match self.lang {
                Language::English => "The submitted form values could not render this template. Check the snippet's variables and form fields.".into(),
                Language::Indonesian => "Nilai formulir yang dikirim tidak dapat merender templat ini. Periksa variabel dan isian formulir cuplikan.".into(),
                Language::German => "Die übermittelten Formularwerte konnten diese Vorlage nicht rendern. Prüfen Sie Variablen und Formularfelder des Snippets.".into(),
            },
            (_, "form submission values are invalid or incomplete") => match self.lang {
                Language::English => "The form submission was missing a value or contained a value that failed validation.".into(),
                Language::Indonesian => "Formulir tidak memiliki nilai wajib atau berisi nilai yang gagal divalidasi.".into(),
                Language::German => "Bei der Formularübermittlung fehlte ein Wert oder ein Wert hat die Prüfung nicht bestanden.".into(),
            },
            (_, "form helper output stream did not close") => match self.lang {
                Language::English => "The form helper did not finish its response. Check the GUI helper process and try again.".into(),
                Language::Indonesian => "Proses bantuan formulir tidak menyelesaikan responsnya. Periksa proses pembantu GUI lalu coba lagi.".into(),
                Language::German => "Der Formularhelfer hat seine Antwort nicht abgeschlossen. Prüfen Sie den GUI-Hilfsprozess und versuchen Sie es erneut.".into(),
            },
            _ => completion.to_owned(),
        }
    }

    pub fn trace_trigger_matched(&self) -> &'static str {
        match self.lang {
            Language::English => "Trigger matched",
            Language::Indonesian => "Pemicu cocok",
            Language::German => "Trigger erkannt",
        }
    }

    pub fn trace_focus_admission(&self) -> &'static str {
        match self.lang {
            Language::English => "Focus admission",
            Language::Indonesian => "Pemeriksaan fokus",
            Language::German => "Fokusfreigabe",
        }
    }

    pub fn trace_backend_selected(&self) -> &'static str {
        match self.lang {
            Language::English => "Backend selected",
            Language::Indonesian => "Backend dipilih",
            Language::German => "Backend ausgewählt",
        }
    }

    pub fn trace_erase_acknowledgement(&self) -> &'static str {
        match self.lang {
            Language::English => "Erase/replacement acknowledgement",
            Language::Indonesian => "Konfirmasi penghapusan/penggantian",
            Language::German => "Bestätigung für Löschen/Ersetzen",
        }
    }

    pub fn trace_insertion_attempted(&self) -> &'static str {
        match self.lang {
            Language::English => "Insertion attempted",
            Language::Indonesian => "Penyisipan dicoba",
            Language::German => "Einfügen versucht",
        }
    }

    pub fn trace_privacy_and_limits(&self) -> &'static str {
        match self.lang {
            Language::English => "Matched policy decisions and output transactions are recorded without typed or inserted text. Output acknowledgement does not prove the target application displayed the text; unmatched typing is never recorded. Use Matcher preview to explain a test input.",
            Language::Indonesian => "Keputusan kebijakan atas pemicu yang cocok dan transaksi keluaran dicatat tanpa teks yang diketik atau disisipkan. Konfirmasi keluaran tidak membuktikan aplikasi tujuan menampilkan teks; ketikan yang tidak cocok tidak pernah dicatat. Gunakan Pratinjau pencocok untuk menjelaskan teks uji.",
            Language::German => "Erkannte Richtlinienentscheidungen und Ausgabetransaktionen werden ohne getippten oder eingefügten Text erfasst. Eine Ausgabebestätigung beweist nicht, dass die Zielanwendung den Text angezeigt hat; nicht erkannte Eingaben werden nie protokolliert. Die Matcher-Vorschau erklärt Testeingaben.",
        }
    }

    pub fn trigger_suggestions(&self) -> &'static str {
        match self.lang {
            Language::English => "Available trigger suggestions",
            Language::Indonesian => "Saran pemicu yang tersedia",
            Language::German => "Verfügbare Trigger-Vorschläge",
        }
    }

    pub fn form_test_values(&self) -> &'static str {
        match self.lang {
            Language::English => "Test this form with sample values",
            Language::Indonesian => "Uji formulir dengan nilai contoh",
            Language::German => "Formular mit Beispielwerten testen",
        }
    }

    pub fn form_template_rendered(&self) -> &'static str {
        match self.lang {
            Language::English => "Rendered template",
            Language::Indonesian => "Templat hasil render",
            Language::German => "Gerenderte Vorlage",
        }
    }

    pub fn form_preview_privacy(&self) -> &'static str {
        match self.lang {
            Language::English => "Values and rendered text stay in this editor session.",
            Language::Indonesian => {
                "Nilai dan teks hasil render hanya tersimpan di sesi editor ini."
            }
            Language::German => "Werte und gerenderter Text bleiben in dieser Editorsitzung.",
        }
    }

    pub fn cursor_placement(&self, position: usize) -> String {
        match self.lang {
            Language::English => format!("Cursor placement: character {position}"),
            Language::Indonesian => format!("Posisi kursor: karakter {position}"),
            Language::German => format!("Cursorposition: Zeichen {position}"),
        }
    }

    pub fn template_error(&self, error: &str) -> String {
        match self.lang {
            Language::English => format!("Template error: {error}"),
            Language::Indonesian => format!("Kesalahan templat: {error}"),
            Language::German => format!("Vorlagenfehler: {error}"),
        }
    }

    pub fn match_explanation(&self, would_expand: bool) -> &'static str {
        match (self.lang, would_expand) {
            (Language::English, true) => "Why this input would expand",
            (Language::English, false) => "Why this input would not expand",
            (Language::Indonesian, true) => "Mengapa teks ini akan diperluas",
            (Language::Indonesian, false) => "Mengapa teks ini tidak akan diperluas",
            (Language::German, true) => "Warum diese Eingabe expandiert würde",
            (Language::German, false) => "Warum diese Eingabe nicht expandiert würde",
        }
    }

    pub fn protection_uncertified(&self) -> &'static str {
        match self.lang {
            Language::English => "Not compositor-certified",
            Language::Indonesian => "Belum disertifikasi untuk kompositor",
            Language::German => "Für diesen Compositor nicht zertifiziert",
        }
    }

    pub fn fleet_layers(&self) -> &'static str {
        match self.lang {
            Language::English => "Fleet layers",
            Language::Indonesian => "Lapisan armada",
            Language::German => "Flotten-Ebenen",
        }
    }

    pub fn not_checked(&self) -> &'static str {
        match self.lang {
            Language::English => "Not checked",
            Language::Indonesian => "Belum diperiksa",
            Language::German => "Nicht geprüft",
        }
    }

    /// A readable label for a backend's state. The raw
    /// `implementation/availability/permission` triple is still shown
    /// verbatim next to it so the GUI stays a faithful mirror of
    /// `wayexpand doctor` rather than paraphrasing it away.
    pub fn backend_state(&self, state: BackendState) -> &'static str {
        match (self.lang, state) {
            (Language::English, BackendState::Available) => "Ready",
            (Language::Indonesian, BackendState::Available) => "Siap",
            (Language::English, BackendState::RequiresPermission) => "Needs permission",
            (Language::Indonesian, BackendState::RequiresPermission) => "Perlu izin",
            (Language::English, BackendState::Implemented) => "Not probed",
            (Language::Indonesian, BackendState::Implemented) => "Belum diperiksa",
            (Language::English, BackendState::Unavailable) => "Unavailable",
            (Language::Indonesian, BackendState::Unavailable) => "Tidak tersedia",
            (Language::English, BackendState::NotImplemented) => "Not implemented",
            (Language::Indonesian, BackendState::NotImplemented) => "Belum diimplementasikan",
            (Language::German, BackendState::Available) => "Bereit",
            (Language::German, BackendState::RequiresPermission) => "Berechtigung nötig",
            (Language::German, BackendState::Implemented) => "Nicht geprüft",
            (Language::German, BackendState::Unavailable) => "Nicht verfügbar",
            (Language::German, BackendState::NotImplemented) => "Nicht implementiert",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_failure_reasons_are_translated() {
        let labels = [
            (
                "command timed out before output",
                "Perintah kehabisan waktu sebelum keluaran",
                "Zeitüberschreitung des Befehls vor der Ausgabe",
            ),
            (
                "command output exceeded policy limit",
                "Keluaran perintah melebihi batas kebijakan",
                "Befehlsausgabe überschritt das Richtlinienlimit",
            ),
            (
                "command result became stale",
                "Hasil perintah dibuang setelah input berubah",
                "Befehlsergebnis nach Eingabeänderung verworfen",
            ),
            (
                "command context was no longer safe",
                "Hasil perintah dibuang karena konteks tidak lagi aman",
                "Befehlsergebnis verworfen, weil der Kontext nicht mehr sicher war",
            ),
            (
                "form requires verified replacement",
                "Cuplikan ini meminta isian formulir, tetapi rute aktif tidak dapat memverifikasi pemicu di kursor. Gunakan rute input-method dengan penggantian terverifikasi, atau hapus penanda `{{field:...}}`.",
                "Dieses Snippet fragt Formularfelder ab, aber die aktive Route kann den Trigger am Cursor nicht verifizieren. Verwenden Sie eine Input-Method-Route mit verifizierter Ersetzung oder entfernen Sie den Marker `{{field:...}}`.",
            ),
            (
                "form values could not render the template",
                "Nilai formulir yang dikirim tidak dapat merender templat ini. Periksa variabel dan isian formulir cuplikan.",
                "Die übermittelten Formularwerte konnten diese Vorlage nicht rendern. Prüfen Sie Variablen und Formularfelder des Snippets.",
            ),
            (
                "form submission values are invalid or incomplete",
                "Formulir tidak memiliki nilai wajib atau berisi nilai yang gagal divalidasi.",
                "Bei der Formularübermittlung fehlte ein Wert oder ein Wert hat die Prüfung nicht bestanden.",
            ),
            (
                "form helper output stream did not close",
                "Proses bantuan formulir tidak menyelesaikan responsnya. Periksa proses pembantu GUI lalu coba lagi.",
                "Der Formularhelfer hat seine Antwort nicht abgeschlossen. Prüfen Sie den GUI-Hilfsprozess und versuchen Sie es erneut.",
            ),
            (
                "input arrived before output",
                "Diblokir karena ada input baru sebelum penggantian",
                "Blockiert, weil vor dem Ersetzen weitere Eingabe eintraf",
            ),
        ];

        for (key, indonesian, german) in labels {
            assert_eq!(
                Strings::new(Language::Indonesian).trace_completion(key),
                indonesian
            );
            assert_eq!(Strings::new(Language::German).trace_completion(key), german);
            assert_ne!(Strings::new(Language::English).trace_completion(key), key);
        }
    }
}
