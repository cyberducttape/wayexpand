# WayExpand

[![Release](https://img.shields.io/github/v/release/cyberducttape/wayexpand?label=release)](https://github.com/cyberducttape/wayexpand/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Text-Expansion, gebaut für Wayland statt nachträglich daran angepasst.**

Tippen Sie einen kurzen Trigger wie `;;hello` und er wird zu einem
gespeicherten Snippet — Signaturen, Runbook-Befehle, Ticket-Antworten,
Boilerplate-Code, Datumsangaben oder alles andere, was Sie regelmäßig neu
eintippen. Geschrieben in Rust; die Expansion-Engine ist strikt von der
Eingabe-Erfassung und Text-Injektion getrennt, die als austauschbare,
explizit ausgewählte Backends pro Wayland-Protokoll implementiert sind
(`input-method-v2`, `wlroots virtual-keyboard`, `libei`/EIS) — statt einer
X11-Implementierung mit nachträglich angeflanschter Wayland-Unterstützung.

*[English](README.md) | Deutsch*

> **Kompatibilitätsangaben sind präzise, nicht optimistisch formuliert.**
> Die Kern-Engine, das TOML-Konfigurationsformat und die CLI/JSON-Verträge
> sind stabil (siehe [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)). Die
> Unterstützung der Desktop-Backends hängt vom Compositor ab, und mehrere
> Erfassungs-/Ausgabe-Pfade sind noch **experimentell**. Kein Compositor ist
> derzeit durch automatisierte End-to-End-Tests zertifiziert; siehe die
> [Support-Matrix](docs/SUPPORT_MATRIX.md). `wayexpand doctor` zeigt, was
> Ihre eigene Sitzung tatsächlich unterstützt, bevor Sie sich darauf verlassen.

## Warum nicht einfach Espanso oder AutoKey?

| | **WayExpand** | Espanso | AutoKey |
|---|---|---|---|
| Wayland-Eingabepfad | Native Backends pro Protokoll (`input-method-v2`, `wlroots virtual-keyboard`, `libei`/EIS), explizit ausgewählt | XTest über XWayland, oder ein Wayland-Modus mit engerer Compositor-Unterstützung | Nur X11/XTest — kein nativer Wayland-Pfad |
| Architektur | Matching-Engine und Injektions-Backend sind getrennte Crates hinter einem Trait; eine Backend-Lücke blockiert nie den Matcher | Ein Rust-Binary, Backend-Auswahl ist intern | Python, an GTK gebunden |
| Konfigurationssicherheit | Parse-then-swap: eine fehlerhafte Konfiguration wird abgelehnt, bevor sie die laufende ersetzt | Reload ersetzt die Konfiguration; Validierung ist impliziter | Reload ersetzt die Konfiguration |
| Sensible Felder | Matching wird automatisch in Passwortfeldern ausgesetzt, wenn das Backend den Fokus meldet (siehe [SECURITY.md](SECURITY.md)) | Nicht explizit modelliert | Nicht modelliert |
| GUI | Native egui-App: Suche, Live-Vorschau, Diagnose, Espanso-Import, atomares Speichern, begrenzte Undo-Historie | Keine (YAML-Dateien) | Native GTK-Oberfläche |
| Diagnose | `wayexpand doctor` meldet den tatsächlichen Zustand jedes Backends (`Implemented`/`RequiresPermission`/`Unavailable`) samt Begründung, plus nicht-mutierende Protokoll-Tests | Eingeschränkt | Eingeschränkt |
| Telemetrie | Keine — kein Konto, keine Cloud, niemals | Keine | Keine |

Diese Tabelle behauptet nicht, dass WayExpand heute in jeder Dimension
überlegen ist — Espanso hat aktuell insbesondere eine breitere
Compositor-Abdeckung von Haus aus. Der Unterschied ist architektonisch:
WayExpand behandelt "welches Wayland-Protokoll unterstützt dieser
Compositor tatsächlich" als Frage, die die Software Ihnen beantworten kann
(`wayexpand doctor`), statt etwas, das Sie erst merken, wenn Tastenanschläge
stillschweigend nicht expandieren.

## Schnellstart

**Ubuntu (PPA):**

```bash
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update && sudo apt install wayexpand
wayexpand doctor          # zeigt, was Ihr Compositor tatsächlich unterstützt
wayexpand-gui             # Snippets grafisch verwalten
```

Auf Debian ist der Ubuntu-PPA nicht vorgesehen. Nutzen Sie dort den
Quellcode- oder Release-Installationsweg. Für Arch ist ein PKGBUILD
vorbereitet, aber noch nicht im AUR eingereicht; bauen Sie es aus dem
Repository. Für Fedora existiert noch kein Copr-Repository.

**Aus Quellen:**

```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
./scripts/install-user.sh          # baut Release-Binaries, installiert nach ~/.local/bin
wayexpand doctor
```

Beide Installer sind standardmäßig nicht-destruktiv (kein Dienst wird
aktiviert oder gestartet, bis `--enable` übergeben wird) und verweigern die
Ausführung als root.

## Konfiguration

### Einfaches Snippet

```toml
[[expansion]]
trigger = ";hello"
replacement = "Hello, world!"
description = "A friendly greeting"
```

Bearbeiten Sie `~/.config/wayexpand/expansions.toml` direkt oder über
`wayexpand-gui`.

### Template-Variablen

```toml
[[expansion]]
trigger = ";sig"
replacement = """Beste Grüße,
{{username}}
—
{{date}} um {{time}}"""
```

Verfügbare Variablen: `{{date}}`, `{{time}}`, `{{datetime}}`,
`{{date+3d}}` (relative Offsets: Tage/Wochen/Stunden/Minuten),
`{{username}}`, `{{hostname}}`, `{{unix_timestamp}}`, `{{newline}}`,
`{{tab}}`, `{{cursor}}` (Cursor-Platzierung nach der Expansion).

### App-Filter

```toml
[[expansion]]
trigger = ";bug"
replacement = "BUG: [describe the issue]"
app_filter = ["vim", "nvim", "code"]
```

Ein Snippet mit `app_filter` **matcht nie**, wenn Fenster-Tracking auf dem
laufenden Compositor nicht verfügbar ist — es matcht nicht etwa überall,
sondern schlägt gezielt fehl ("fail closed"). Aktuell nur auf KDE Plasma
(KWin) implementiert.

### Dynamische Befehle

```toml
[[expansion]]
trigger = ";sys"
replacement = "[uname output]"
[expansion.command]
program = "uname"
args = ["-s", "-r", "-m"]
timeout_ms = 500
cache_ms = 0
```

Das konfigurierte Programm wird direkt ausgeführt (keine Shell-Syntax).
`timeout_ms` begrenzt die Laufzeit, `cache_ms` cached das Ergebnis (`0` =
kein Caching).

## Daemon und Backends

| Situation | Backend | Dienst | Status |
|---|---|---|---|
| Compositor bietet `input-method-v2`/virtual-keyboard | `--source=input-method` | `wayexpand-input-method.service` | Experimentell (siehe [Support-Matrix](docs/SUPPORT_MATRIX.md)) |
| Compositor bietet keins davon (z. B. KWin/KDE Plasma bis 6.6) | `--source=evdev --backend=libei` | `wayexpand-evdev.service` | Experimentell, aktive-Sitzplatz-ACL standardmäßig (optionale `input`-Gruppe), **keine Sensible-Feld-Erkennung** |
| wlroots-Compositor (Sway, Hyprland), wenn `--source=input-method` benötigte Tasten verliert | `--source=evdev --backend=wlroots` | — (Daemon manuell starten) | Experimentell, gleiche Berechtigungsoptionen und Einschränkungen wie oben |
| Fenster-Tracking (`app_filter`) | KWin-Scripting-Bridge | — | KDE/KWin implementiert, Zertifizierung noch ausstehend; siehe [Support-Matrix](docs/SUPPORT_MATRIX.md) |

`--source=evdev` funktioniert compositor-unabhängig, liest aber
Tastatur-Events direkt vom Kernel und erkennt daher **keine** Passwortfelder
— lesen Sie [SECURITY.md](SECURITY.md), bevor Sie es aktivieren. Der Installer
verwendet standardmäßig aktive-Sitzplatz-ACLs ohne dauerhafte Gruppenmitgliedschaft.
Die breitere Legacy-Variante kann ausdrücklich gewählt werden:

```bash
sudo ./scripts/install-evdev-permissions.sh --dry-run
sudo ./scripts/install-evdev-permissions.sh
# Legacy-Fallback mit dauerhafter, breiterer input-Gruppenmitgliedschaft:
sudo ./scripts/install-evdev-permissions.sh --access=input-group
```

```bash
systemctl --user status wayexpand-evdev.service
wayexpand status
```

Der `wayexpand-evdev.service`-Dienst startet nach Fehlern mit
`Restart=on-failure` automatisch neu. Das `libei`-Backend speichert das
Portal-Einwilligungstoken; nach der ersten Freigabe benötigen Neustarts
normalerweise keinen neuen Berechtigungsdialog. Nach einem Compositor-Neustart
oder Portal-Problem kann ein manueller Neustart weiterhin hilfreich sein:
`systemctl --user restart wayexpand-evdev.service`.

Details zur Problembehebung: [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md).

## Sicherheit

Siehe [SECURITY.md](SECURITY.md) für das vollständige Bedrohungsmodell.
Kurz zusammengefasst:

- Keine Shell-Interpretation — konfigurierte Befehle sind ein Programm plus
  Argumentliste, keine Shell-Syntax
- Benutzer-eigene Konfigurationsdateien müssen Modus 0600 haben; verwaltete
  root-eigene Dateien dürfen lesbar sein, aber nicht durch Gruppe/Andere
  veränderbar werden
- Control-Socket ist auf ein privates, eigentümer-verifiziertes Verzeichnis
  beschränkt
- Matching wird automatisch in Passwortfeldern ausgesetzt — außer bei
  `--source=evdev`, das dafür keine Signal-Quelle hat

## Stabilität

Die Kern-Engine, das TOML-Schema und ausgewählte CLI/JSON-Ausgaben
(`list --json`, `preview --json`, `status --json`, `doctor --json`) sind
stabil dokumentierte Verträge. Siehe
[docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) für die exakte Feldliste
jedes Vertrags — das ist genauer als eine pauschale
"stabil"-Behauptung und wird bei jeder Änderung mitgepflegt.

## GUI

```bash
wayexpand-gui
```

Suche, Kategorie-Filter, Live-Vorschau (die niemals automatisch ein
befehlsgestütztes Snippet ausführt — dafür gibt es einen expliziten
"Run once"-Button), Metadaten-Bearbeitung, Espanso-Import mit Vorschau,
Diagnose-Panel, atomares Speichern, begrenzte Undo-Historie (32 Schritte),
Daemon-Pause/Fortsetzen. Acht Farbschemata inklusive Retro-Terminal-Themes
(VT220-Grün, IBM-3270-Blau, Commodore 64) — siehe
[docs/COLOR_PACKS.md](docs/COLOR_PACKS.md). Schriftskalierung (0,8×–2,0×)
und WCAG-2.1-AA-Kontrast für Barrierefreiheit — siehe
[docs/CUSTOMIZATION.md](docs/CUSTOMIZATION.md).

## CLI-Befehle

```bash
wayexpand validate ~/.config/wayexpand/expansions.toml
wayexpand test ";hello"                    # simuliert Matching, druckt das Ergebnis
wayexpand test-hotkey Ctrl+Alt+E           # testet eine Hotkey-Bindung
wayexpand preview ";hello"                 # rendert das Template für einen Trigger
wayexpand list                             # alle Snippets auflisten
wayexpand search "email"                   # Snippets durchsuchen
wayexpand doctor                           # Backend-/Sitzungsbericht
wayexpand status                           # Daemon-Status
```

`test`/`preview` injizieren niemals Text in eine andere Anwendung. Für ein
einfaches (Template-)Snippet ist `test` ein reiner, folgenloser Trockenlauf.
Für ein **befehlsgestütztes** Snippet führt `test` das konfigurierte
Programm trotzdem real aus, um dessen Ausgabe zu erzeugen — es gibt keine
Möglichkeit, die Ausgabe eines Befehls zu sehen, ohne ihn auszuführen.
Prüfen Sie `program`/`args`, bevor Sie `test` gegen eine Konfiguration
ausführen, die Sie nicht selbst geschrieben haben.

## Entwicklung

- **Rust 1.87+** (`rustup update`)
- Wayland-Entwicklungsbibliotheken (Ubuntu/Debian: `libwayland-dev
  libxkbcommon-dev`; Fedora: `wayland-devel libxkbcommon-devel`)

```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
cargo build --release
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Details zu Projektstruktur und PR-Ablauf: [CONTRIBUTING.md](CONTRIBUTING.md)
(Englisch).

## Lizenz

[MIT](LICENSE)
