# WayExpand

[![CI](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml/badge.svg)](https://github.com/itchyitchy123/wayexpand/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

WayExpand is a Wayland-first text-expansion daemon. The expansion engine is platform-independent; input capture and text insertion are separate backends.

It is designed for privacy-conscious desktop automation: configuration is
validated before activation, control surfaces are permission-checked, and the
daemon never logs typed text or snippet contents.

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
- isolated libei/EIS output backend using UTF-8 text insertion
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

## Try it

```sh
cargo test
cargo run -p wayexpand -- test ';;hello' expansions.toml
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

For a user-local installation with systemd units:

```sh
./scripts/install-user.sh
```

The installer builds release binaries, installs them under `~/.local/bin`,
installs both user units, registers the graphical editor with the desktop
application menu, and creates the example configuration only when one does not
already exist. It does not enable or start a service automatically.

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
