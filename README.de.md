# WayExpand

[![Release](https://img.shields.io/github/v/release/cyberducttape/wayexpand?label=release)](https://github.com/cyberducttape/wayexpand/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Text-Expansion für Wayland.**

*[English](README.md) | Deutsch*

WayExpand ersetzt kurze Trigger wie `;;hello` durch Snippets. Die Kern-Engine,
das TOML-Konfigurationsformat und die CLI/JSON-Verträge sind stabil. Die
Desktop-Backends hängen vom Compositor ab; mehrere Erfassungs- und
Injektionspfade sind experimentell und derzeit ist kein Compositor durch die
automatisierte End-to-End-Zertifizierung freigegeben. Prüfen Sie vor dem
produktiven Einsatz `wayexpand doctor` und die
[Support-Matrix](docs/SUPPORT_MATRIX.md).

## Schnellstart

### Ubuntu

```bash
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update
sudo apt install wayexpand
wayexpand doctor
wayexpand-gui
```

### Aus Quellen

```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
./scripts/install-user.sh
wayexpand doctor
```

Die Installer sind standardmäßig nicht-destruktiv und aktivieren keinen Dienst,
solange nicht ausdrücklich `--enable` verwendet wird. Die Installation als
root wird verweigert.

## Konfiguration

Ein einfaches Snippet:

```toml
[[expansion]]
trigger = ";hello"
replacement = "Hello, world!"
description = "A friendly greeting"
```

Benutzerkonfigurationen müssen den Modus `0600` haben:

```sh
install -m 600 /dev/null ~/.config/wayexpand/expansions.toml
```

Danach können Sie die Datei bearbeiten oder `wayexpand-gui` verwenden.
Templates unterstützen unter anderem `{{username}}`, `{{hostname}}`,
`{{date}}`, `{{time}}`, `{{datetime}}`, `{{newline}}` und
`{{cursor}}`. Datum und Zeit werden in UTC berechnet.

Weitere Felder, Limits, Befehle, App-Filter und Fleet-Konfiguration:
[docs/CONFIGURATION_LIMITS.md](docs/CONFIGURATION_LIMITS.md),
[docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) und
[docs/FLEET_CONFIG.md](docs/FLEET_CONFIG.md).

## Backends und Sicherheit

Die automatische Auswahl bleibt konservativ. IBus bzw.
`input-method-v2` oder ein explizit gewählter Backend-Pfad kann je nach
Sitzung verfügbar sein. Evdev ist ein Kompatibilitäts-Fallback: Es liest
Tastaturereignisse direkt von `/dev/input`, besitzt kein Signal für
Passwortfelder und ist deshalb für sicherheitskritische Umgebungen sorgfältig
zu bewerten. Der Installer verwendet standardmäßig aktive-Sitzplatz-ACLs;
`--access=input-group` ist ein breiterer Legacy-Fallback.

```bash
sudo ./scripts/install-evdev-permissions.sh --dry-run
sudo ./scripts/install-evdev-permissions.sh
# Legacy-Fallback:
sudo ./scripts/install-evdev-permissions.sh --access=input-group
```

Systemd-Benutzerdienste stellen den Daemon-Lebenszyklus bereit, zertifizieren
aber nicht automatisch die Desktop-Funktionalität. Diese hängt vom gewählten
Backend und der Support-Matrix ab.

- [Support-Matrix](docs/SUPPORT_MATRIX.md)
- [Backend- und Sicherheitsdetails](docs/BACKENDS.md)
- [Bedrohungsmodell](THREAT_MODEL.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [GUI-Handbuch](docs/GUI.md)

## CLI

```bash
wayexpand validate ~/.config/wayexpand/expansions.toml
wayexpand test ";hello"
wayexpand preview ";hello"
wayexpand doctor
wayexpand status
```

`test` und `preview` injizieren keinen Text in andere Anwendungen. Bei
befehlsgestützten Snippets kann `test` das konfigurierte Programm ausführen;
prüfen Sie `program` und `args` vor der Verwendung.

## Entwicklung und weitere Dokumentation

Die technische Referenz ist bewusst zentral auf Englisch gepflegt:

- [Getting Started](docs/GETTING_STARTED.md)
- [Configuration and compatibility](docs/COMPATIBILITY.md)
- [Operations](docs/OPERATIONS.md)
- [Packaging](docs/PACKAGING.md)
- [Development](docs/DEVELOPMENT.md)
- [Migration from Espanso](docs/MIGRATION_FROM_ESPANSO.md)
- [Complete documentation index](docs/DOCUMENTATION_INDEX.md)

Beiträge und Fehlerberichte:
[CONTRIBUTING.md](CONTRIBUTING.md) und
[GitHub Issues](https://github.com/cyberducttape/wayexpand/issues).

## Lizenz

[MIT](LICENSE)
