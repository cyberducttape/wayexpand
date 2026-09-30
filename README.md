# WayExpand

[![CI](https://github.com/cyberducttape/wayexpand/actions/workflows/ci.yml/badge.svg)](https://github.com/cyberducttape/wayexpand/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/cyberducttape/wayexpand?label=release)](https://github.com/cyberducttape/wayexpand/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](https://www.rust-lang.org/)

## Local text expansion for Linux desktops

Type a short trigger and get the text you use every day — locally, without
sending anything to the cloud.

```text
;;sig   →  Stephan Loesevitz
           Cyberdeck Labs

;;date  →  YYYY-MM-DD

;;ip    →  Server: prod-api-03
           Status: investigating
```

WayExpand is a text expander for Linux desktops: signatures, support replies,
dates, command output, boilerplate, and team snippets wherever your desktop
backend supports safe expansion.

No cloud. No account. No telemetry.

Built to be boring to operate: local by default, explicit about permissions,
and conservative when configuration or command execution looks unsafe.

It includes a GUI, a scriptable CLI, Espanso import, templates, and fleet
policy support. The goal is simple: type less and keep your snippets local.

> **Status:** WayExpand is usable today, but desktop integration is still
> compositor-dependent. Run `wayexpand doctor` on your own session before
> enabling a backend. No compositor is currently certified by automated
> end-to-end tests; see the [support matrix](docs/SUPPORT_MATRIX.md).

> **Important limitations:** WayExpand sees committed text, not active IME
> preedit/composition. Complete CJK/IBus/Fcitx, dead-key, and Compose sequences
> before expecting a trigger to match. Its intended scope is direct keyboard
> input and committed-text workflows; no desktop is yet E2E certified.
> App-filtered snippets require focused-window tracking; that integration is
> currently available only through the KWin bridge, and fails closed
> elsewhere. If either capability is essential
> to your workflow, verify it with `wayexpand doctor` on the exact desktop
> session before deployment.

## Install

On Ubuntu:

```sh
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update
sudo apt install wayexpand
wayexpand doctor
wayexpand-gui
```

On Debian, use the source/release installation path for now; the Launchpad PPA
targets Ubuntu series, not Debian releases.

New GitHub releases are built to include installable Debian and RPM packages
for x86_64, plus a native aarch64 release archive. Existing releases may not
contain these assets. Fedora/Copr and AUR repositories are not yet published;
see [Packaging](docs/PACKAGING.md) for the verified options.

For those release assets, install the downloaded local package with
`sudo apt install ./wayexpand_*_amd64.deb` or
`sudo dnf install ./wayexpand-*.x86_64.rpm`. These are per-release packages,
not auto-updating Debian/Fedora repositories.

From source:

```sh
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
./scripts/install-user.sh
wayexpand doctor
wayexpand-gui
```

The normal installers do not run as root, enable services automatically, grant
raw-input permissions, or accept portal consent for you.

## Enable WayExpand and create your first snippet

```sh
wayexpand doctor       # inspect this desktop's available paths
wayexpand setup        # choose/configure a path; review any security prompt
wayexpand status       # confirm the selected service is running
wayexpand-gui          # create and save a snippet
```

In the GUI, create a snippet with trigger `;;hello` and replacement
`Hello, world!`, save it, then type `;;hello` in a plain-text field. You can
also use `wayexpand-ui` for the terminal editor. `wayexpand test ';;hello'`
checks matching in the config; it does not test desktop capture or injection.

Automatic setup never grants raw-input access by itself. In particular,
evdev can observe typing in password fields and is not password-safe. Read the
backend warning shown by setup and run `wayexpand doctor` before relying on
expansion. If no suitable route is available, setup reports that instead of
silently enabling one.

## Desktop compatibility

Desktop integration remains compositor- and session-dependent. No compositor
is currently certified by automated end-to-end tests. Active IME/preedit
composition is unsupported, and app-filtered snippets currently have a window
tracker only on KDE/KWin. Check your exact session before relying on either
feature:

```sh
wayexpand doctor
wayexpand explain-backend
```

See the [support matrix](docs/SUPPORT_MATRIX.md) and
[certification matrix](docs/CERTIFICATION_MATRIX.md) for evidence and known
limits.

## Security and operational safety

WayExpand is designed to run as the unprivileged desktop user, work offline,
avoid telemetry, keep configuration files owner-checked, execute command
snippets without a shell, and suspend matching in sensitive fields when the
selected input backend can report them. The evdev fallback is different: it
requires explicit raw input-event access and an additional
`--allow-evdev-sensitive-fields` acknowledgement because it cannot detect
password fields.

The daemon also uses bounded command queues, output limits, timeouts, process
group cleanup, and organization policy that can either audit or enforce
restrictions. These controls are intended to make everyday operation
predictable, not to claim that WayExpand is a security sandbox.

## Migrating from Espanso

```sh
wayexpand import espanso ~/.config/espanso/match/base.yml > imported.toml
```

The GUI defaults to a merge that preserves current snippets and reports
duplicates/conflicts; replacement is a separate explicit action. Review the
CLI's migration report or GUI preview for unmapped features before applying.
See
[Migrating from Espanso](docs/MIGRATION_FROM_ESPANSO.md).

## More information

- [Getting started and troubleshooting](docs/GETTING_STARTED.md)
- [Security and threat model](SECURITY.md) · [Threat model](THREAT_MODEL.md)
- [Backend details](docs/BACKENDS.md) · [Support matrix](docs/SUPPORT_MATRIX.md)
- [Packaging status](docs/PACKAGING.md)
- [Migration from Espanso](docs/MIGRATION_FROM_ESPANSO.md)
- [Fleet deployment](docs/FLEET_CONFIG.md) · [Documentation index](docs/DOCUMENTATION_INDEX.md)

Non-English documentation: [Deutsch](README.de.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, PR checklist,
and project structure. CI runs formatting, tests, Clippy, shellcheck, installer
checks, systemd validation, packaging checks, and dependency advisories.

## License

[MIT](LICENSE)
