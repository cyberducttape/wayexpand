# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

### Added

- **GUI language support**: English and German, with in-app switching (🌐 button), `LANG` environment auto-detection, and persisted preference. See [docs/LANGUAGE_SUPPORT.md](docs/LANGUAGE_SUPPORT.md).
- **GUI color packs**: six selectable themes including retro monochrome terminal styles (Classic Green, Classic Amber, Classic White), a Retro 80s Neon theme, and a High Contrast accessibility theme, alongside the existing Default theme. Preference persists across restarts. See [docs/COLOR_PACKS.md](docs/COLOR_PACKS.md).
- **German README** (`README.de.md`).
- `ExpansionConfig::propagate_case`: opt-in case propagation — typing a trigger in `UPPERCASE` or `Capitalized` form applies the same casing to the replacement. Exposed as a checkbox in the GUI editor. See [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md).
- **Date math in templates**: `{{date}}`, `{{time}}`, and `{{datetime}}` accept a relative offset, e.g. `{{date+3d}}`, `{{date-1w}}`, `{{time+5h}}`, `{{datetime+90m}}`.
- **`{{cursor}}` placement marker**: positions the cursor after typing the replacement instead of leaving it at the end (e.g. `replacement = "(){{cursor}}"`). Implemented on the libei and wlroots backends (both synthesize a Left key); has no effect on input-method-v2, which has no protocol-level way to move the cursor after committing text. Exposed in the GUI's template-variable picker; `wayexpand test/preview --json` report it as `cursor_offset`.
- **Undo last expansion**: `settings.undo_chord` (e.g. `"Ctrl+Z"`) reverts the most recent expansion — erasing the replacement and typing the original trigger back — if pressed with no other keystroke in between. Disabled unless configured; exposed in the GUI Settings dialog.

### Fixed

- **Correctness**: word-boundary matching could silently fail open and expand an embedded trigger (e.g. `hello:sig`) when `max_buffer_chars` was small enough that the character preceding the trigger had already been evicted from the matcher's rolling buffer. Now fails closed when that context is unknown rather than assuming no boundary violation.
- **Reliability**: the daemon's own shutdown could hang and be forcibly `SIGKILL`ed by systemd when using the libei backend against some desktop-portal implementations, because dropping the injector's Tokio runtime can block until its background tasks reach a safe stopping point. The injector's drop is now moved to a detached thread so a hang there can no longer delay the daemon's own exit or the control-socket cleanup that a clean restart depends on.
- **Security**: the KWin window-tracker backend wrote its helper script to a predictable `/tmp` path derived only from the process PID, which a local attacker able to guess the upcoming PID could pre-empt with a symlink to overwrite an arbitrary file the user owns. The path now includes a random component and is created with `O_CREAT|O_EXCL` semantics, refusing to write through anything already present at that path.
- **Correctness**: the clipboard backend's `get_clipboard` skipped its `xsel` fallback whenever the `xclip` binary was entirely absent (rather than merely failing), silently defeating clipboard restoration on `xsel`-only installs.
- **Correctness**: the clipboard backend now refuses to initialize when no X11 `DISPLAY` is available, rather than constructing successfully and then silently failing (or misdirecting keystrokes via XWayland) on every paste/erase — this backend synthesizes input via `xdotool`/XTest and has no Wayland-native equivalent.
- GUI: the color-pack selector previously had no effect on the toolbar, sidebar, snippet list, or any other custom-painted widget, because the frame's palette was still hardcoded to the Default pack regardless of the selected color pack.
- GUI: the Diagnostics, Import, Settings, and unsaved-changes dialogs were left entirely untranslated when switching to German.
- README.de.md: fixed stray non-German text in the dynamic-command section.

### Changed

- README.md: updated version badge and release banner from the stale v0.2.1 to v1.0.0, and linked the new customization and German documentation.
- `TextInjector` now requires `Send`, needed for the shutdown-hang fix above; all existing backends already satisfied this.
- `TextInjector` gained a `move_cursor_left` method (default no-op; non-breaking for any external implementation) for `{{cursor}}` support.

## [1.0.0] - 2026-09-17

This is the first stable release. WayExpand is now recommended for production use on Wayland desktops. The API and configuration format are stable within 1.x versions. See [COMPATIBILITY.md](docs/COMPATIBILITY.md) for stability guarantees.

### Added

- **Stability guarantees** for CLI exit codes, JSON output shapes, and TOML config schema (see [COMPATIBILITY.md](docs/COMPATIBILITY.md))
- **Security audit** with verification of command execution, config permissions, socket security, and D-Bus integration
- **CI build hardening**: Vendor all dependencies for offline Launchpad builds; explicit Rust toolchain configuration for modified HOME environments
- **Comprehensive documentation**: COMPATIBILITY.md for third-party integrations, SECURITY_AUDIT.md for compliance verification

### Fixed

- GitHub Actions CI: Set `RUSTUP_TOOLCHAIN=stable` in installer for isolated HOME environments
- Launchpad Debian builds: Restored `CARGO_NET_OFFLINE=true` and added vendored dependency support
- Debian packaging: Added debian/source/format and debian/cargo-checksum.json for dh-cargo compatibility

### Changed

- Release tagging: v0.2.1 build/CI hardening → ready for 1.0.0 certification
- Compositor support status moved from "Experimental" to "Supported" for tested backends per [INTEGRATION_TESTING.md](docs/INTEGRATION_TESTING.md) protocol
- Control socket and daemon security model formally documented in [SECURITY_AUDIT.md](docs/SECURITY_AUDIT.md)

### Known Limitations

- **Window tracking (app_filter)**: KDE Plasma only. wlroots (`wlr-foreign-toplevel-management`) implementation planned for 1.1
- **Sensitive field detection**: Not available with `--source=evdev` backend; see [SECURITY.md](SECURITY.md) for tradeoff documentation
- **Preedit/IME composition**: Not supported; tracked as future enhancement per [INTEGRATION_TESTING.md](docs/INTEGRATION_TESTING.md)

## [0.2.0] - 2026-09-16

### Added

- Experimental `--source=evdev` capture backend (`wayexpand-backend-evdev`),
  a compositor-agnostic fallback that reads keyboard events directly from
  `/dev/input` for compositors without `zwp_input_method_manager_v2` or
  `zwp_virtual_keyboard_manager_v1` support (for example KWin/KDE Plasma).
  It has no sensitive-field signal and requires `input` group membership;
  see `docs/SECURITY.md` and `docs/SUPPORT_MATRIX.md`.
- `scripts/install-evdev-permissions.sh` and `udev/71-wayexpand-evdev.rules`,
  a separate, explicit, root-requiring step to grant `--source=evdev`
  permission (never run automatically by the user installers).
- `systemd/wayexpand-evdev.service` for running `--source=evdev` with a
  `--backend=libei` output as a user service, installed and removed by the
  user installers and uninstaller. It deliberately does not auto-restart:
  the libei backend requests desktop-control consent per connection, and
  restarting on failure re-shows that portal dialog faster than a person can
  answer it.
- `ei_keyboard` fallback in the libei output backend for EIS servers that
  never offer `ei_text` (observed with xdg-desktop-portal-kde on KWin 6.6).
  Characters are looked up in the keymap the server itself supplies, so the
  fallback is layout-dependent; a replacement containing a character the
  current layout cannot produce is rejected before anything is typed rather
  than partially inserted.
- Refreshed GUI visual design: layered surfaces, a typographic scale,
  custom-painted snippet rows with status and command badges, primary and
  destructive button styles, status-colored messages, and a light/dark theme
  toggle.

### Fixed

- User services no longer fail to start with `218/CAPABILITIES`.
  `CapabilityBoundingSet=`, `PrivateDevices=`, `ProtectClock=`,
  `ProtectKernelLogs=`, and `ProtectKernelModules=` each try to shrink the
  capability bounding set, which requires `CAP_SETPCAP` that a
  `systemctl --user` service never has.
- Configuration and control-socket directory trust checks stop walking
  ancestors once a directory owned by the current user is confirmed, instead
  of continuing to `/`. Under a systemd sandbox the real root owner of `/` is
  remapped to the overflow uid and was rejected as untrusted.
- Expansions no longer lose the characters matching the trigger's final
  keys (`:hello` produced "Hell from Wayland!", `:sig` produced "Reards,").
  evdev capture is non-exclusive and a match fires on key-down, so those
  keys are still physically held when injection starts; the compositor read
  our duplicate press as auto-repeat and our release as cancelling the
  physical one. `EvdevSource` now tracks held keys and the daemon waits
  (bounded) for them to be released before injecting. This also prevents a
  replacement being uppercased when the trigger needed Shift.
- Synthesized keystrokes in the libei `ei_keyboard` fallback are paced and
  flushed per character, and the trigger erase is flushed before typing
  begins, matching what other synthetic-input tools do. Note this was not
  what caused the dropped characters above.
- Replaced GUI glyphs that egui's bundled fonts do not cover and which
  rendered as missing-glyph boxes, including the `＋` on the New and Create
  snippet buttons.

### Changed

- Repositioned the project description and Cargo keywords around
  Wayland-native text expansion rather than accessibility tooling.

## [0.1.0] - 2026-09-10

### Added

- Published support matrix, contribution policy, pull request checklist, and
  privacy-safe bug-report template.
- Continuous dependency advisory auditing in CI.
- JSON output for safe expansion simulation through `test --json`.
- Optional installer service activation with explicit `--enable` and
  `--service` controls, plus clear `sudo` and dependency guidance.
- Installer now rejects all root execution and prints the GUI as an explicit
  post-install next step.
- `doctor` now reports capture readiness separately from backend implementation
  status and fails clearly when a Wayland session has no usable input source.
- Isolated release smoke test covering clean installation, validation, preview,
  hotkey resolution, diagnostics, and configuration permissions.
- Tagged Linux x86_64 release workflow with bundled deployment files and
  SHA256 checksums.
- A clearer GUI empty state, filtered-search state, library counts, and
  contextual editor guidance.
- Explicit delete confirmation in the GUI while retaining undo recovery.
- `wayexpand test-hotkey` to resolve configured hotkeys without executing
  actions or requiring a compositor.
- `wayexpand backup` to create a private, non-overwriting configuration
  backup.
- Hotkeys in `list --json` output for inventory and deployment tooling.
- Normalized key-chord parsing for `Ctrl`, `Alt`, `Shift`, and `Super`
  bindings, including common modifier aliases.
- Validated hotkey action configuration with duplicate detection and bounded
  command arguments and timeouts.
- Wayland input-method key normalization with modifier detection.
- Direct hotkey action execution without shell interpolation.
- `wayexpand doctor --json` for service checks, monitoring, and fleet
  diagnostics.

### Changed

- The GUI now surfaces unsaved work, runtime controls, and command-backed
  expansion risk closer to the relevant workflow.
- Hotkey actions are disabled automatically while sensitive input is focused.
- Hotkey action failures are isolated and logged without terminating the
  daemon.
- Operations documentation now describes machine-readable health checks and
  service monitoring.

### Security

- Hotkey programs receive no stdin and discard stdout and stderr.
- Hotkey execution is bounded by the configured timeout.
- Systemd services validate configuration before startup and write logs to the
  journal with stable service identifiers.

### Reliability

- User services now treat SIGTERM as a clean stop and retain bounded restart
  behavior.
- Installer idempotence, workspace tests, Clippy, systemd verification, and
  systemd security analysis remain covered by the release checks.

[Unreleased]: https://github.com/itchyitchy123/wayexpand/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/itchyitchy123/wayexpand/releases/tag/v0.1.0
