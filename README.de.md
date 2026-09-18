# WayExpand

> Ein datenschutzfreundlicher, Wayland-nativer Text-Expander für Linux

**WayExpand** verwandelt kurze Tastenkombinationen in lange Text-Ersetzungen. Tippen Sie `;sig` um eine Signatur einzufügen, `;today` um das aktuelle Datum zu erhalten – oder schreiben Sie eigene Snippets in unter 30 Sekunden.

[![Release](https://img.shields.io/github/v/release/itchyitchy123/wayexpand?label=release)](https://github.com/itchyitchy123/wayexpand/releases)
[![MIT License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

*[English](README.md) | Deutsch*

---

## Features

- **🔒 Datenschutz-fokussiert** — Keine Telemetrie, keine Cloud-Abhängigkeiten
- **⚡ Wayland-nativ** — Funktioniert auf Sway, Hyprland, KDE Plasma und GNOME
- **🛡️ Sicher** — Keine Shell-Interpretation, getestete Berechtigungen, Sockets mit Umask
- **📝 Einfache Konfiguration** — TOML-basiert, einfach zu lesen und zu schreiben
- **🎯 Gezielt** — Snippets können auf bestimmte Apps begrenzt werden
- **⚙️ Dynamisch** — Führe Befehle zur Laufzeit aus und nutze Template-Variablen
- **🌍 Mehrsprachig** — Englisch und Deutsch (weitere Sprachen willkommen)

---

## Installation

### Ubuntu/Debian (empfohlen)

```bash
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update
sudo apt install wayexpand
```

### Arch Linux (AUR)

```bash
yay -S wayexpand
```

### Aus Quellen bauen

```bash
git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand
cargo build --release
./scripts/install-user.sh
./scripts/install-evdev-permissions.sh
```

---

## Schnellstart

### 1. Konfiguration öffnen

WayExpand wird konfiguriert über `~/.config/wayexpand/expansions.toml`:

```bash
wayexpand-gui
```

Dies öffnet den grafischen Editor. Sie können die Datei auch direkt bearbeiten:

```bash
$EDITOR ~/.config/wayexpand/expansions.toml
```

### 2. Ein einfaches Snippet hinzufügen

```toml
[[expansion]]
trigger = ";hello"
replacement = "Hello, world!"
description = "A friendly greeting"
```

### 3. Daemon starten

Die Auswahl der Backend hängt von Ihrem Kompositor ab:

**KDE Plasma 6.6+**
```bash
systemctl --user enable --now wayexpand-evdev.service
```

**Sway/Hyprland (input-method-v2)**
```bash
systemctl --user enable --now wayexpand-input-method.service
```

**GNOME (input-method-v2)**
```bash
systemctl --user enable --now wayexpand-input-method.service
```

### 4. Testen

Öffnen Sie einen Texteditor und tippen Sie `;hello`. Es sollte zu "Hello, world!" expandiert werden.

---

## Konfiguration

### Basis-Snippet

```toml
[[expansion]]
trigger = ";email"
replacement = "my.email@example.com"
description = "Meine E-Mail-Adresse"
tags = ["kontakt", "e-mail"]
category = "Kontakt"
enabled = true
match_mode = "Immediate"  # oder "WordBoundary"
```

### Template-Variablen

Verwenden Sie eingebaute Variablen in Ersetzungen:

```toml
[[expansion]]
trigger = ";sig"
replacement = """Beste Grüße,
{{username}}
–
{{date}} at {{time}}"""
```

Verfügbare Variablen:
- `{{date}}` — UTC-Datum (YYYY-MM-DD)
- `{{time}}` — UTC-Zeit (HH:MM:SS)
- `{{datetime}}` — Beide kombiniert
- `{{username}}` — Aktueller Benutzer
- `{{hostname}}` — Lokaler Hostname
- `{{unix_timestamp}}` — Unix-Zeitstempel
- `{{newline}}` — Zeilenumbruch
- `{{tab}}` — Tabulatorzeichen

### Bedingte Expansion (App-Filter)

Aktivieren Sie Snippets nur in bestimmten Anwendungen:

```toml
[[expansion]]
trigger = ";bug"
replacement = "BUG: [describe the issue]"
app_filter = ["vim", "nvim", "code"]  # Nur in diesen Apps
```

Verfügbar auf KDE Plasma 6.6+. Auf anderen Compositoren deaktiviert.

### Dynamische Befehle

Führe beliebige Programme aus statt Texte zu ersetzen:

```toml
[[expansion]]
trigger = ";sys"
replacement = "[uname output]"  # Fallback-Text
[expansion.command]
program = "uname"
args = ["-s", "-r", "-m"]
timeout_ms = 500
cache_ms = 0
```

- **timeout_ms** — Max. Zeit für Programmausführung
- **cache_ms** — Ergebnis zwischenspeichern (0 = kein Caching)

---

## Daemons und Backends

WayExpand unterstützt mehrere Text-Injektions-Methoden, je nach Kompositor:

| Kompositor | Backend | Service | Status |
|---|---|---|---|
| **KDE Plasma 6.6+** | evdev + libei | `wayexpand-evdev.service` | ✅ Unterstützt |
| **KDE Plasma 6.7+** | evdev + input-method-v2 | `wayexpand-input-method.service` | ✅ Empfohlen |
| **GNOME (Mutter)** | input-method-v2 | `wayexpand-input-method.service` | ✅ Empfohlen |
| **Sway/Hyprland** | input-method-v2 | `wayexpand-input-method.service` | ✅ Empfohlen |
| **Andere (X11)** | Nicht unterstützt | — | ❌ |

### Daemon Status überprüfen

```bash
systemctl --user status wayexpand-evdev.service
# oder
wayexpand status
```

### Problembehebung

Siehe [docs/wiki/Troubleshooting.md](docs/wiki/Troubleshooting.md) für detaillierte Lösungen.

---

## Sicherheit

WayExpand wurde nach einem Bedrohungsmodell überprüft. Siehe [docs/SECURITY_AUDIT.md](docs/SECURITY_AUDIT.md).

**Sicherheits-Highlights:**
- ✅ Keine Shell-Interpretation (Befehle sind keine Shell-Syntax)
- ✅ Konfigurationsdatei-Berechtigungen werden erzwungen (Modus 0600)
- ✅ Control-Sockets mit Umask 0o077
- ✅ Keine D-Bus-Namenskollisionen (eindeutige Service-Namen)

---

## Stabilität

WayExpand 1.0.0 bietet Stabilitätsgarantien für:

- ✅ **CLI-Ausstiegscodes** — Definiert und dokumentiert
- ✅ **JSON-Ausgabeformen** — `wayexpand test`, `doctor`, etc.
- ✅ **TOML-Konfigurationsschema** — Rückwärts- und vorwärtskompatibel
- ✅ **Kommandobefehl-Ausgaben** — Stabil in Minor-Versionen

Siehe [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) für vollständige Details.

---

## Roadmap 1.1

- [ ] **wlroots Window Tracker** — Für Sway/Hyprland Window-Tracking (15-20 Stunden)
- [ ] **GUI Performance** — Async-Loading für "Use current app"-Button (1-2 Stunden)
- [ ] **Zusätzliche Backends** — X11 Input-Extension, weitere Wayland-Protokolle

Siehe [docs/WLROOTS_WINDOW_TRACKER_PLAN.md](docs/WLROOTS_WINDOW_TRACKER_PLAN.md) für technische Details.

---

## GUI-Modus

Öffnen Sie den graphischen Editor:

```bash
wayexpand-gui
```

**Funktionen:**
- 🖱️ Visuelle Snippet-Verwaltung
- 📋 Vorschau mit Live-Expansion
- 🎯 "Use current app"-Button zur App-Erkennung
- 💾 Atomare Speicherung
- ↩️ Rückgängig (32-fach)
- 📥 Espanso-Import
- 🔧 Diagnose-Panel

---

## CLI-Befehle

```bash
# Konfiguration validieren
wayexpand validate ~/.config/wayexpand/expansions.toml

# Expansion testen
wayexpand test ";hello"

# Hotkey-Bindung testen
wayexpand test-hotkey Ctrl+Alt+E

# Vorschau
wayexpand preview ";hello" ";;hallo"

# Alle Snippets auflisten
wayexpand list

# Schnellsuche
wayexpand search "email"

# Diagnose
wayexpand doctor

# Daemon-Status
wayexpand status
```

---

## Daemon-Kontrolle

```bash
# Daemon pausieren
systemctl --user stop wayexpand-evdev.service

# Daemon fortsetzen
systemctl --user start wayexpand-evdev.service

# Neustart
systemctl --user restart wayexpand-evdev.service

# Aktivieren bei Login
systemctl --user enable wayexpand-evdev.service
```

---

## Entwicklung

### Abhängigkeiten

- **Rust 1.70+** — `rustup update`
- **Wayland-Entwicklungsbibliotheken**

Ubuntu/Debian:
```bash
sudo apt install libwayland-dev libxkbcommon-dev
```

Fedora:
```bash
sudo dnf install wayland-devel libxkbcommon-devel
```

### Bauen

```bash
cargo build --release
```

### Tests

```bash
cargo test
```

### Formatierung

```bash
cargo fmt
```

---

## Beitrag leisten

Beiträge sind willkommen! Bitte:

1. Forken Sie das Repository
2. Erstellen Sie einen Feature-Branch (`git checkout -b feature/my-feature`)
3. Committen Sie Ihre Änderungen (`git commit -am 'Add feature'`)
4. Pushen Sie zum Branch (`git push origin feature/my-feature`)
5. Öffnen Sie einen Pull Request

Siehe [CONTRIBUTING.md](CONTRIBUTING.md) für detaillierte Richtlinien.

---

## Lizenz

MIT License — Siehe [LICENSE](LICENSE) für Details.

---

## Fragen?

- 📖 Dokumentation: [docs/](docs/)
- 🐛 Probleme: [GitHub Issues](https://github.com/itchyitchy123/wayexpand/issues)
- 💬 Diskussionen: [GitHub Discussions](https://github.com/itchyitchy123/wayexpand/discussions)

---

**Gebaut mit ❤️ für Wayland-Nutzer überall.**
