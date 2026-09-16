# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

### Added

### Fixed

### Changed

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
