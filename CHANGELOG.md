# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/itchyitchy123/wayexpand/compare/main...HEAD
