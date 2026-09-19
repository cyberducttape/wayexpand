# WayExpand

[![CI](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml/badge.svg)](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/itchyitchy123/wayexpand?label=release)](https://github.com/itchyitchy123/wayexpand/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.87%2B-orange.svg)](https://www.rust-lang.org/)

**Text expansion built for Wayland, rather than adapted to it.**

Type a short trigger like `;;hello` and it becomes a saved snippet —
signatures, runbook commands, ticket responses, boilerplate code, dates, or
anything else you retype every day. Written in Rust, with the expansion
engine kept entirely separate from input capture and text injection, which
are pluggable, explicitly-selected backends per Wayland protocol
(`input-method-v2`, `wlroots virtual-keyboard`, `libei`/EIS) rather than one
X11-shaped implementation with Wayland support bolted on.

> **Compatibility is stated precisely, not optimistically.** The core
> engine, TOML config format, and CLI/JSON contracts are stable (see
> [COMPATIBILITY.md](docs/COMPATIBILITY.md)). Desktop backend support is
> compositor-dependent and several capture/output paths are still
> **experimental** — the [support matrix](docs/SUPPORT_MATRIX.md) states
> exactly what's been verified versus implemented-but-untested, and
> `wayexpand doctor` tells you what your own session can actually use
> before you rely on it. **KDE Plasma (KWin 6.6+)** and 
> Sway, Hyprland, and river have a wlroots tracking scaffold awaiting
> active-protocol and compositor certification.

![WayExpand snippet dashboard](docs/archive/wiki/assets/snippets-dashboard.png)

## What's New (v1.2 in progress)

- ⚠️ **Wlroots window tracking:** Scaffolded for Sway, Hyprland, and river; active tracking and certification remain.
- 🎯 **Conservative backend auto-selection:** Use `wayexpand backend select --explain` to inspect the decision and its tradeoffs.
- 🔐 **Portal permission persistence:** One-time libei consent prompt with seamless reconnection. Token stored securely at `~/.config/wayexpand/libei-portal-token`.
- 🧰 **Portal recovery controls:** Inspect or forget the stored restoration token with `wayexpand portal status` and `wayexpand portal reset`.
- 🔒 **P0 security fixes:** Config reload preserves password-field protection and user-paused state. Evdev terminator re-insertion working correctly.
- 📚 **Clarity on compositor support:** [SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) documents what's verified vs. in-progress. [GETTING_STARTED.md](docs/GETTING_STARTED.md) provides per-compositor quick starts.
- 🧪 **Workspace test suite:** Run `cargo test --locked --workspace` for the current test set across the core engine, CLI, daemon, wlroots integration, and all backends.

**Get started:** See [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md) for your desktop.

### Release status

WayExpand v1.1.2 is a released build with stable core/configuration and CLI
contracts. “Released” does not mean every compositor path is production-ready:
input-method-v2, libei/EIS, evdev, and window tracking remain
compositor-dependent, and several are experimental or awaiting independent
certification. Check [SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) and run
`wayexpand doctor` before relying on a backend for daily or sensitive work.

## Why not just use Espanso or AutoKey?

|                          | **WayExpand**                                                        | Espanso                                        | AutoKey                             |
| ------------------------ | --------------------------------------------------------------------- | ----------------------------------------------- | ------------------------------------ |
| Wayland input path       | Native per-protocol backends (`input-method-v2`, wlroots virtual-keyboard, libei/EIS), selected conservatively or explicitly | XTest via XWayland, or a Wayland mode with narrower compositor support | X11/XTest only — no native Wayland path |
| Architecture             | Matching engine and injection backend are separate crates behind a trait; a backend gap never blocks the matcher | Single Rust binary, backend selection is internal | Python, GTK-bound |
| Config safety            | Parse-then-swap: a malformed config is rejected before it ever replaces the live one | Reload replaces config; validation is more implicit | Reload replaces config |
| Sensitive-field handling | Matching suspends automatically in password fields on backends that report focus (see [SECURITY.md](SECURITY.md)) | Not modeled explicitly | Not modeled |
| GUI                      | Native egui app: search, live preview, diagnostics, Espanso import, atomic saves, bounded undo | None (YAML files) | Native GTK editor |
| Diagnostics              | `wayexpand doctor` reports per-backend state (`Implemented` / `RequiresPermission` / `Unavailable`) with the *reason*, plus non-mutating protocol probes | Limited | Limited |
| Telemetry                | None — no account, no cloud, no phone-home, ever | None | None |

This table is not a claim that WayExpand is strictly better in every
dimension today — Espanso in particular has broader out-of-the-box
compositor coverage right now. The distinction is architectural: WayExpand
treats "which Wayland protocol does this compositor actually implement" as
a first-class question the software can answer for you (`wayexpand doctor`),
rather than something you find out by watching keystrokes silently fail to
expand.

## Architecture

```
                    ┌─────────────────────────────┐
                    │   wayexpand-core (engine)   │
                    │  trie matcher · templates   │
                    │  config validation · undo   │
                    └───────────┬─────────────────┘
                                │  TextInjector / InputSource traits
                ┌───────────────┼──────────────────────────┐
                │               │                          │
      ┌─────────▼──────┐ ┌──────▼──────────┐   ┌──────────▼─────────┐
      │ input-method-v2│ │ evdev capture   │   │   libei / EIS      │
      │  (capture+type)│ │(compositor-     │   │ (portal-mediated)  │
      │ (exclusive,    │ │ agnostic, needs │   │     output)        │
      │  XKB-based)    │ │ `input` group)  │   └────────────────────┘
      └────────────────┘ └─────────────────┘
                                │
                    ┌───────────▼──────────────┐
                    │ wlroots virtual-keyboard│
                    │    (output only)         │
                    └──────────────────────────┘
```

Every backend implements a small trait (`InputSource` for capture,
`TextInjector` for output) and is selected conservatively or explicitly at
daemon startup (`--source=`, `--backend=`). Use
`wayexpand backend select --explain` to inspect automatic selection. A backend that
doesn't exist for your compositor is a documented gap, not a runtime
surprise: `app_filter`-scoped snippets **fail closed** (never match) rather
than matching everywhere when window tracking isn't available, the same
philosophy the matcher applies to `word-boundary` mode when its rolling
buffer has already evicted the context it needs.

## Quick start

**Ubuntu/Debian (PPA):**

```sh
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update && sudo apt install wayexpand
wayexpand doctor          # see what your compositor actually supports
wayexpand-gui             # manage snippets graphically
```

**Arch Linux (packaging preview):**

```sh
# The repository PKGBUILD is not yet an official AUR submission.
git clone https://github.com/itchyitchy123/wayexpand
cd wayexpand
makepkg -si
```

The PKGBUILD currently targets `x86_64` only. AUR publication and aarch64
support require clean-chroot build and upgrade/removal verification first.

> **Fedora/RHEL note:** No official Copr repository exists yet. Build from source using the RPM spec file
> in the repository, or use the release tarball with `install-release.sh`. Community contributions welcome.

**From source** (any distro, no packaging required):

```sh
git clone https://github.com/itchyitchy123/wayexpand
cd wayexpand
./scripts/install-user.sh          # builds release binaries, installs to ~/.local/bin
wayexpand doctor
```

Both installers are non-destructive by default (no service is enabled or
started until you pass `--enable`) and refuse to run as root — see
[Installation](#installation) below for the full picture, including
`--source=evdev` for compositors with no `input-method-v2` or
virtual-keyboard support (KWin/KDE Plasma, as of KWin 6.6).

## Screenshots

The dashboard above shows the snippet library: search, category filters,
per-row status toggles, and a live preview pane that never auto-executes a
command-backed snippet (see [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)).

**Diagnostics** reports every backend's actual state
(`Implemented`/`RequiresPermission`/`Unavailable`) plus non-mutating
protocol probes, in one view — the same information `wayexpand doctor
--json` exposes to scripts and health checks:

![WayExpand diagnostics](docs/archive/wiki/assets/diagnostics.png)

Eight color packs ship in the GUI, including retro terminal themes (VT220
green, IBM 3270 blue, Commodore 64) alongside the default — see
[docs/COLOR_PACKS.md](docs/COLOR_PACKS.md) and
[docs/CUSTOMIZATION.md](docs/CUSTOMIZATION.md). Full accessibility support
(font scaling 0.8×–2.0×, WCAG 2.1 AA contrast, keyboard focus indicators) is
documented alongside them.

## Features

**Core engine**
- Unicode-safe suffix matching via a trie; longest-match trigger families
  (`:a` and `:address` coexist correctly)
- Optional `word-boundary` matching for triggers that must not fire inside
  larger words — fails closed if its context window has been evicted rather
  than guessing
- `propagate_case`: typing a trigger as `UPPERCASE` or `Capitalized`
  applies the same casing to the replacement (validated against generated
  case variants, not just the literal trigger, so it can't silently collide
  with another snippet)
- Template variables (`{{date}}`, `{{date+3d}}`, `{{username}}`,
  `{{cursor}}` placement, and more) rendered without shelling out
- Bounded, cached, direct-program expansions for dynamic content (no shell
  interpretation — a program and argument list, executed directly)
- `app_filter`: restrict a snippet to specific applications, backed by a
  focused-window tracker (KDE Plasma via KWin's scripting bridge today)
- Undo-last-expansion via a configurable key chord on exclusive input sources;
  disabled with evdev because its physical hotkey also reaches the application

**Safety and reliability**
- Parse-then-swap config reloads: a malformed config is rejected before it
  replaces the live one, both from the CLI and from the daemon watching the
  file
- Matching suspends automatically in password fields on backends that
  report sensitive focus
- The control socket is permission-checked and confined to a private,
  owner-verified directory
- Every backend, queue, and child process is bounded — see
  [SECURITY.md](SECURITY.md) for the full threat model

**Interfaces**
- Native Wayland-capable GUI (`wayexpand-gui`): search, live preview,
  metadata editing, Espanso import with a diff-style preview, diagnostics,
  atomic saves, bounded undo history, daemon pause/resume
- Dependency-light terminal UI (`wayexpand-ui`) for SSH sessions and
  systems without a GPU presentation path
- Scriptable CLI with stable `--json` output for `list`, `preview`,
  `status`, and `doctor` — see [COMPATIBILITY.md](docs/COMPATIBILITY.md)
  for exactly which fields are guaranteed
- One-shot, atomic `set-enabled`/`set-mode` commands so a script or
  integration never needs to hand-edit TOML

**Migration**
- Espanso import (`wayexpand import espanso`) converts YAML matches to
  TOML, skips unsupported non-string matches with a warning, and never
  touches the source file

See [docs/FOR_SYSADMINS.md](docs/FOR_SYSADMINS.md) for
production-ready snippets (SSL certs, logrotate, systemd units, firewall
rules, Docker, deployment scripts) and enterprise deployment guidance if you
want a running start rather than an empty library.

For large snippet libraries, see [PERFORMANCE_TUNING.md](docs/PERFORMANCE_TUNING.md).
For backend failures, follow the [Troubleshooting Checklist](docs/TROUBLESHOOTING_CHECKLIST.md).

## Installation

WayExpand's deployment depends on your compositor and its Wayland protocol support. The valid backend combinations are:

**Option 1: input-method-v2 (single unified backend)**

For compositors advertising `zwp_input_method_manager_v2` (verify support for your compositor):
```sh
systemctl --user enable --now wayexpand-input-method.service
```

Pros:
- Simpler setup (one backend handles capture+output)
- Respects sensitive-field signals in password fields
- Requires no special permissions

Cons:
- Experimental — unsupported key events (Escape, arrows, F-keys) may not pass through (see [support matrix](docs/SUPPORT_MATRIX.md))

**Option 2: evdev + libei/wlroots (split capture/output)**

For compositors without input-method-v2 (e.g. KDE Plasma/KWin 6.6+), or when Option 1 loses keys you need. Output goes through `--backend=libei` on desktops with a RemoteDesktop portal (KDE Plasma, GNOME) or `--backend=wlroots` on wlroots compositors; the shipped unit uses libei:
```sh
sudo ./scripts/install-evdev-permissions.sh --dry-run   # preview first
sudo ./scripts/install-evdev-permissions.sh             # then apply
systemctl --user enable --now wayexpand-evdev.service
```

Pros:
- Better keyboard fidelity (all keys pass through)
- evdev provides compositor-independent capture; output still requires a compatible libei/EIS portal or virtual-keyboard protocol

Cons:
- **Requires `input` group membership** — grants raw keyboard access to **all keystrokes** system-wide, not just WayExpand's
- **No password-field protection** — matching is never suspended in password fields
- Experimental — read [SECURITY.md](SECURITY.md) before enabling

This unit does not auto-restart on failure by design. A portal session may
need fresh user consent after revocation or an expired restoration token, and
automatic restarts could create permission-dialog storms. Restart it yourself
after a compositor restart or portal hiccup:
`systemctl --user restart wayexpand-evdev.service`.

**Which should I choose?**

Run `wayexpand doctor` after installation to see which backends your compositor supports:
```sh
wayexpand doctor
```

If you're on KDE Plasma, use **Option 2** (evdev) — it's the only tested path with good keyboard fidelity.

On a compositor where `wayexpand doctor` confirms input-method-v2 support, try **Option 1** first; if you lose keyboard input (Escape/arrows), switch to Option 2 if your compositor supports it, or accept the limitation.

**Granting evdev permission**

Granting the permission is a separate, explicit, root-requiring step the installers never run for you. Understand what `input` group membership means before proceeding (see [SECURITY.md](SECURITY.md) for details).

```sh
sudo ./scripts/install-evdev-permissions.sh --dry-run   # preview first
sudo ./scripts/install-evdev-permissions.sh             # then apply
```

Once `wayexpand doctor` reports capture readiness:

```sh
systemctl --user enable --now wayexpand-evdev.service
```

This unit does not auto-restart on failure by design. A portal session may
need fresh user consent after revocation or an expired restoration token, and
automatic restarts could create permission-dialog storms. Restart it yourself
after a compositor restart or portal hiccup:
`systemctl --user restart wayexpand-evdev.service`.

Neither installer runs as root, enables a service automatically, or
overwrites an existing configuration. `install-user.sh` builds from source;
`install-release.sh` installs the prebuilt binaries from a downloaded
[release](https://github.com/itchyitchy123/wayexpand/releases) tarball. To
remove an installation, run `./scripts/uninstall-user.sh` (`--purge` also
deletes the configuration directory).

## Using the CLI

```sh
wayexpand test ';;hello' expansions.toml           # simulate matching, print the result
wayexpand test ';;hello' --json expansions.toml
wayexpand preview ':today' expansions.toml         # render templates without matching
wayexpand list expansions.toml
wayexpand validate expansions.toml
wayexpand validate --fleet --json                    # CI-friendly merged validation
wayexpand set-enabled ':sig' off expansions.toml   # atomic, single-snippet edit
wayexpand set-mode ':sig' word-boundary expansions.toml
wayexpand import espanso ~/.config/espanso/match/base.yml > imported.toml
wayexpand doctor                                   # human-readable backend/session report
wayexpand doctor --json                            # stable schema for health checks
wayexpand backend select --explain                 # explain automatic backend selection
wayexpand fleet status                              # inspect merged fleet layers
wayexpand fleet status --json                        # inspect provenance as JSON
```

When the daemon starts without an explicit config path, it loads the normal
`~/.config/wayexpand/expansions.toml` first and then merges standard fleet
layers from `/etc/wayexpand/snippets.d/`,
`~/.config/wayexpand/snippets.d/`, and
`~/.local/share/wayexpand/packs/`. Explicit `WAYEXPAND_CONFIG` paths and
positional config paths retain single-file behavior. Duplicate triggers or
hotkeys fail closed and are reported as configuration errors.

`test`/`preview` never inject text into another application. For a plain
(template) expansion, `test` is a pure, side-effect-free dry run. For a
**command-backed** expansion, `test` still runs the configured program for
real to produce its output — there's no way to preview a command's output
without running it. Review `program`/`args` before running `test` against
a config you didn't write yourself.

Configuration resource limits are documented in
[`docs/CONFIGURATION_LIMITS.md`](docs/CONFIGURATION_LIMITS.md).

## Documentation

**Start here:** [DOCUMENTATION_INDEX.md](docs/DOCUMENTATION_INDEX.md) — find what you need by use case or role

**Get started:**
- [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md) — installation, first snippet, verify it works
- [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) — common issues, solutions, and diagnostics
- [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) — what's tested vs. experimental per compositor
- [docs/MIGRATION_FROM_ESPANSO.md](docs/MIGRATION_FROM_ESPANSO.md) — if switching from Espanso

**Configuration & customization:**
- [docs/CUSTOMIZATION.md](docs/CUSTOMIZATION.md) — GUI themes, color packs, language support
- [docs/CONFIGURATION_LIMITS.md](docs/CONFIGURATION_LIMITS.md) — resource and safety boundaries

**Enterprise & security:**
- [docs/ORGANIZATION_POLICY.md](docs/ORGANIZATION_POLICY.md) — policy enforcement and fleet management
- [docs/FLEET_CONFIG.md](docs/FLEET_CONFIG.md) — multi-machine deployment (Ansible, Puppet examples)
- [docs/SECRET_MANAGEMENT.md](docs/SECRET_MANAGEMENT.md) — handling sensitive data safely

**Compatibility & design:**
- [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) — which CLI/JSON/config fields are stable
- [docs/BACKENDS.md](docs/BACKENDS.md) — backend protocols and architecture
- [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) — tested combinations per desktop/protocol

**Examples & reference:**
- [docs/FOR_SYSADMINS.md](docs/FOR_SYSADMINS.md) — ready-made snippets and enterprise deployment
- [docs/ANSIBLE_INTEGRATION.md](docs/ANSIBLE_INTEGRATION.md) — fleet deployment playbooks

**For developers:**
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) — building, testing, and contributing

Non-English documentation: [Deutsch](README.de.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, PR
checklist, and project structure. CI runs formatting, the full test suite,
Clippy with warnings denied, shellcheck, installer idempotence in an
isolated home, systemd unit validation, and a separate dependency-advisory
job on every change.

## License

[MIT](LICENSE)
