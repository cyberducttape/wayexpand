# WayExpand

[![CI](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml/badge.svg)](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Text expansion built for Wayland, rather than adapted to it.**

WayExpand is a privacy-first text expander for Linux Wayland desktops. Type a
short trigger like `;;hello` and it replaces it with a saved snippet —
signatures, runbook commands, ticket responses, code, dates, or anything else
you retype often.

Written in Rust, with native Wayland input and output backends, GUI and CLI
snippet management, Espanso import, parse-before-swap configuration reloads,
and local-only operation that never logs your typed text.

Privacy-first • Rust • Wayland-native • GUI + CLI • Espanso import

![WayExpand snippet dashboard](docs/wiki/assets/snippets-dashboard.png)

## Why WayExpand?

- **Wayland-native architecture.** The expansion engine is
  platform-independent; input capture and text insertion are separate,
  explicitly selected backends (`input-method-v2`, wlroots virtual-keyboard,
  libei/EIS, and an evdev capture fallback) rather than an X11 design with
  Wayland bolted on.
- **Local and private by default.** No account, no cloud, no telemetry. The
  daemon never logs typed text or snippet contents, matching is suspended in
  password fields on backends that report them, and control surfaces are
  permission-checked.
- **Explicit over magical.** Configuration is validated before it replaces
  live state, elevated permissions are separate opt-in steps you run
  yourself, and `wayexpand doctor` explains exactly what your compositor does
  and does not support.
- **Both audiences.** Manage snippets in a native GUI without touching TOML,
  or drive the whole thing from the CLI and keep your configuration in Git.

Compositor coverage is still uneven, and that is a property of Wayland rather
than a shipping omission: see the [support matrix](docs/SUPPORT_MATRIX.md) for
what is tested versus experimental, and run `wayexpand doctor` to see which
backends your own session can actually use.

## Documentation

The [project wiki](docs/wiki/README.md) includes a guided installation,
configuration reference, GUI walkthrough with screenshots, operations runbook,
security model, troubleshooting playbook, and contributor release checklist.

## Current milestone

This repository contains a hardened core plus opt-in Wayland source and output
paths:

- Rust workspace with a reusable core engine
- TOML configuration
- Unicode-safe suffix matching with a trie
- malformed configuration fails before replacing live state
- CLI test command
- daemon stdin harness for exercising matching without a compositor
- backend capability reporting via `wayexpand doctor`
- parse-then-swap configuration reloads while the daemon is running
- sensitive-focus events disable matching and clear buffered input
- isolated wlroots virtual-keyboard output backend
- isolated libei/EIS output backend using UTF-8 text insertion, with a
  layout-dependent keysym-synthesis fallback when the EIS server does not
  offer `ei_text`
- isolated input-method-v2 source with timeout-aware daemon integration
- protected Unix control socket with status/reload/stop commands
- longest-match trigger families (for example `:a` and `:address`)
- optional Unicode-aware `word-boundary` matching for triggers that must not
  expand inside larger words
- safe built-in replacement templates and CLI preview/list/validation commands
- bounded direct-program expansions for trusted local system information
- searchable snippet descriptions and tags
- UI-editable immediate and Unicode-aware word-boundary modes
- runtime pause/resume without stopping the daemon
- interactive terminal settings app with live preview and safe editing
- native Wayland-capable graphical editor with diagnostics, settings, import,
  template helpers, duplication, and bounded undo history
- shared normalized key-chord model ready for cross-backend hotkey dispatch
- validated hotkey action declarations with sensitive-focus-aware dispatch

The default daemon mode remains a stdin harness. An explicit
`--source=input-method` mode can use the input-method-v2 source as both the
capture and insertion path; it fails closed for unsupported non-text keys and
for unsafe Unicode Backspace situations. Unsupported grabbed events may be
lost until compositor-specific pass-through is implemented, but do not restart
the daemon. Output backends remain explicit:
`--backend=wlroots` or `--backend=libei`. Input-method startup and transport
loss, plus runtime output-session loss in the stdin harness, are recovered with
bounded backoff. Compositor coverage, preedit support, and ordinary non-text
pass-through still require compositor integration testing.

For compositors that do not advertise `zwp_input_method_manager_v2` or
`zwp_virtual_keyboard_manager_v1` at all (for example KWin/KDE Plasma as of
KWin 6.6), an experimental `--source=evdev` mode reads keyboard events
directly from the kernel instead, paired with an independent
`--backend=wlroots` or `--backend=libei` output. This works on any
compositor but requires `input` group membership and has **no sensitive-field
signal**, so matching is never suspended in password fields; read
[SECURITY.md](SECURITY.md) before enabling it. Granting the required
permission is a separate, explicit, root-requiring step, never run
automatically by the installers above:

```sh
sudo ./scripts/install-evdev-permissions.sh --dry-run   # preview first
sudo ./scripts/install-evdev-permissions.sh             # then apply
```

Once `wayexpand doctor` reports capture readiness, `wayexpand-evdev.service` runs
this combination (`--source=evdev --backend=libei` by default; edit the unit
to `--backend=wlroots` if your compositor supports the wlroots
virtual-keyboard protocol instead):

```sh
systemctl --user enable --now wayexpand-evdev.service
```

Unlike the other two units, this one does not auto-restart on failure. The
`libei` backend connects through the desktop RemoteDesktop portal without
persisting consent (see `crates/backend-libei`), so every connection attempt
shows a fresh permission dialog; auto-restarting would re-show it faster than
you can respond. `enable --now` starts it once so you can grant that consent;
after any later failure (compositor restart, portal hiccup), restart it
yourself with `systemctl --user restart wayexpand-evdev.service`.

## Try it

```sh
cargo test
cargo run -p wayexpand -- test ';;hello' expansions.toml
cargo run -p wayexpand -- test ';;hello' --json expansions.toml
cargo run -p wayexpand -- preview ':today' expansions.toml
cargo run -p wayexpand -- list expansions.toml
cargo run -p wayexpand -- validate expansions.toml
cargo run -p wayexpand -- list --json expansions.toml
cargo run -p wayexpand -- preview ':today' --json expansions.toml
cargo run -p wayexpand -- set-enabled ':sig' off expansions.toml
cargo run -p wayexpand -- set-mode ':sig' word-boundary expansions.toml
cargo run -p wayexpand -- import espanso ~/.config/espanso/match/base.yml > imported.toml
cargo run -p wayexpand-daemon -- expansions.toml
cargo run -p wayexpand-daemon -- --source=input-method expansions.toml
cargo run -p wayexpand -- doctor [config]
cargo run -p wayexpand -- doctor --json [config]
cargo run -p wayexpand -- backend
cargo run -p wayexpand-ui -- expansions.toml
cargo run -p wayexpand-gui -- expansions.toml
```

`test` is the safe dry-run path: it simulates matching and prints the result;
it never injects text into another application. Add `--json` when consuming
the result from CI, scripts, or an editor integration.

For a user-local installation with systemd units, built from source:

```sh
./scripts/install-user.sh
```

Installing from a downloaded [release](https://github.com/itchyitchy123/wayexpand/releases)
tarball instead uses the prebuilt binaries and does not require Cargo:

```sh
tar -xzf wayexpand-*-linux-x86_64.tar.gz
cd wayexpand-*-linux-x86_64
./scripts/install-release.sh
```

Both installers are intentionally non-destructive by default. After reviewing
`wayexpand doctor`, the complete setup can be requested explicitly:

```sh
./scripts/install-user.sh --enable \
  --service=wayexpand-input-method.service
```

Do not run either installer as root or with `sudo`; they install user binaries
and user systemd units and must inherit the desktop user's Wayland
environment. Use `--help` on either script for the supported options.

`install-user.sh` builds release binaries from source before installing them;
`install-release.sh` installs the prebuilt binaries already present in the
extracted tarball. Both install to `~/.local/bin`, install all three user
units, register the graphical editor with the desktop application menu, and create
the example configuration only when one does not already exist. Neither
enables or starts a service automatically unless `--enable` is passed. To
remove an installation made by either script, run
`./scripts/uninstall-user.sh` (add `--purge` to also delete the configuration
directory).

Espanso users can migrate without replacing their existing files. The importer
writes converted TOML to standard output and leaves the source untouched;
unsupported non-string matches are skipped with a warning.

Configuration errors are reported without silently accepting invalid entries. Reloads parse a new configuration completely before swapping it into the running daemon. Replacement contents are intentionally never printed by the daemon harness.

Operational and security constraints are documented in [docs/OPERATIONS.md](docs/OPERATIONS.md) and [SECURITY.md](SECURITY.md). Backend design decisions are tracked in [docs/BACKENDS.md](docs/BACKENDS.md), with the external compositor test matrix in [docs/INTEGRATION_TESTING.md](docs/INTEGRATION_TESTING.md).

The repository runs formatting, workspace tests, and Clippy in CI.
CI also exercises the daemon smoke test, installer idempotence in an isolated
home, shell scripts, and systemd unit validation.
Dependency advisories are checked in a separate CI supply-chain job. The
[support matrix](docs/SUPPORT_MATRIX.md) distinguishes tested behavior from
experimental backends and explicitly records the current pass-through gap.
Tagged releases are built by CI with the locked dependency graph and publish a
Linux x86_64 archive plus SHA256 checksum; see [docs/RELEASING.md](docs/RELEASING.md).

The CLI's `list --json`, `preview --json`, and lifecycle `status --json`
commands provide a machine-readable surface for settings frontends and desktop
integrations; human-readable output remains the default.
`doctor --json` provides a stable health snapshot for systemd checks, shell
monitoring, and fleet diagnostics without running compositor probes.
`set-enabled` and `set-mode` edit a single snippet through an atomic, validated replacement
so a UI or script never needs to rewrite configuration unsafely.

`wayexpand-ui` is the dependency-light terminal settings frontend. The native
Wayland-capable `wayexpand-gui` frontend provides a graphical snippet browser,
search, live preview, metadata editing, bounded direct-program expansion
editing, Espanso import with preview, diagnostics, atomic saves, undo, and
daemon pause/resume controls. Both frontends use the same core model and
control contract.
