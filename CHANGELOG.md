# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

Changes not yet released.

- Make setup recognize an installed IBus component before the IBus daemon has
  reloaded its engine registry, while still requiring the WayExpand IBus
  executable to be present.
- Require the IBus runtime client as well, so automatic setup never selects a
  path whose restart and engine-selection commands cannot run.
- Synchronize CLI and man-page command discovery for fleet status and portal
  token operations.
- Require certification evidence to identify GTK, Qt, and password/PIN-field
  clients before any compositor report can be marked complete.
- Make required certification client coverage declarative in the compositor
  matrix so the evidence tools share one source of truth.
- Record the matrix-derived client markers in each JSON certification report.
- Quote explicit certification CLI paths so binaries in directories containing
  spaces are probed exactly as requested.
- Require compositor evidence to include a healthy doctor result and a valid
  daemon-status snapshot before it can be certified, with regression coverage
  for unhealthy probes.
- Require the status snapshot to explicitly report a running daemon rather than
  accepting an arbitrary JSON object.
- Make certification backend-aware: IBus uses the healthy doctor/IBus probe,
  while daemon-backed paths require a matching running source/backend pair.
- Add certification coverage for matching daemon-backed source/backend metadata.
- Align input-method-v2 certification with the daemon’s actual status backend
  label and cover that status shape in the regression suite.
- Add a manual self-hosted certification workflow covering KDE, GNOME, Sway,
  and Hyprland without treating missing runner drivers as certification.
- Declare the certification runner labels for actionlint so the workflow’s
  self-hosted matrix is linted rather than silently skipped.
- Document the backend-specific certification probe rules, including the IBus
  exception for the daemon control socket.
- Document the healthy-doctor and running-daemon prerequisites for certification
  evidence.
- Make workflow validation shellcheck-safe while checking every GitHub Actions
  workflow, and keep clean-build archive extraction explicitly quoted.
- Reject malformed, unknown, or duplicate compositor-certification results so
  evidence cannot pass because of a typo or conflicting entry.
- Mark the archived desktop-status snapshot as historical so superseded
  verification claims cannot be mistaken for current certification evidence.
- Clarify getting-started desktop labels so implementation status is not
  presented as compositor certification.
- Clarify setup’s Recommended label so detected availability is not described
  as end-to-end verification.
- Add a validated JSON output format to the compositor evidence collector,
  including session metadata, live probes, and per-scenario results.
- Clarify that river and other wlroots sessions have implementation paths but
  are outside the current four-desktop certification matrix.
- Document the JSON evidence workflow alongside the human-readable
  certification instructions.
- Require a valid doctor JSON snapshot before compositor evidence can claim
  certification, while preserving explicit backend health details in the
  record.
- Allow certification runs to bind doctor/status probes to an explicit
  WayExpand binary instead of an unrelated installation in `PATH`.
- Reject unknown IBus factory engine names instead of returning the root object
  path as if it were an engine instance.
- Add an explicit certification evidence status so automation can distinguish
  incomplete runs from runs with observed failures.
- Synchronize the installed man page with the complete CLI command surface.
- Align installer onboarding with compatibility-mode setup instead of telling
  users to choose an unverified production backend manually.
- Add a matrix-driven compositor certification driver contract so real GTK/Qt
  runners can execute every KDE, GNOME, Sway, and Hyprland scenario uniformly.
- Mark the audit checklist accurately: the runner contract is complete, while
  provisioning and executing real compositor drivers remains open.
- Add compatibility-mode setup (`recommended`, `maximum`, and `experimental`)
  so routine onboarding chooses a safe path without requiring protocol
  knowledge; retain `explain-backend` for expert diagnostics.
- Add `wayexpand certify [--json]` with explicit verified, available,
  unsupported, authorization-required, and not-run states. Certification
  refuses to claim desktop support until a real compositor/client harness has
  exercised typing, focus, password, restart, and reload scenarios.
- Expose backend limitations in human and JSON diagnostics and document the
  KDE, GNOME, Sway, and Hyprland certification matrix.
- Require compositor version/backend metadata and all scenarios to pass in
  certification evidence; explicit failures can no longer produce a passing
  evidence record.
- Validate certification backend/compositor combinations and check the KDE,
  GNOME, Sway, and Hyprland matrix contract in CI.
- Use a named IBus release-mask constant shared by the adapter and regression
  tests; the official ibus-rs binding was evaluated but rejected because it
  adds a mandatory native libdbus build dependency.

- Harden IBus integration: ignore key-release events, isolate engine instances
  per input context, and surface live configuration reload failures.
- Make libei portal-token persistence and token location explicit, including
  systemd/XDG-safe paths and tests for disabled persistence.
- Unify organization-policy parsing and trust validation across the daemon,
  CLI, and diagnostics; doctor health now reflects policy and backend state.
- Apply the same root-owned organization policy to IBus as to the daemon, and
  make setup/doctor refuse to recommend policy-disallowed input paths.
- Make certification selection policy-aware so reports cannot claim a blocked
  IBus or automatic output backend is the active safe mode.
- Make doctor readiness and human backend diagnostics hide policy-disallowed
  IBus paths instead of reporting them as available to try.
- Correct the daemon’s published organization-policy test fixture to use the
  authoritative `input-method-v2` backend name.
- Make release preparation update and verify AppStream and IBus component
  versions so SemVer tags satisfy the release workflow.
- Increase the bounded systemd task budget to leave room for the daemon’s
  command and hotkey workers; the previous limit could crash-loop services with
  `EAGAIN` while starting the hotkey worker.
- Make worker-thread startup fail closed to a synchronous fallback with a
  warning instead of panicking when an external task limit is too restrictive.
- Make optional KWin tracker probing and nonce generation return availability
  errors instead of panicking on thread or entropy resource failures.
- Apply organization-policy filtering to every doctor capture/output readiness
  path, not only IBus.
- Reconcile developer and roadmap documentation with the current automatic
  resolver, typed CLI exit categories, and uncertified compositor status.
- Make compositor certification paths and required scenarios derive from the
  checked-in matrix, preventing backend, scenario, and documentation drift.
- Make JSON doctor health reject automatic backend selections disallowed by
  organization policy.
- Mark policy-disallowed protocol paths as unsupported in certification output
  instead of reporting their session probes as available.
- Align the roadmap’s backend-selection strategy with the shared resolver and
  setup compatibility modes.
- Make `wayexpand certify --json` enumerate the same required scenarios as the
  checked-in compositor certification matrix.
- Add a CI contract test that rejects certification reports with missing
  scenarios or a false `certified` claim while checks remain unverified.
- Prevent setup from activating the libei service under a wlroots-only
  organization policy.
- Enforce organization backend policy at daemon startup so explicit and
  automatic selections cannot bypass the shared policy decision.
- Centralize resolved-source backend identity mapping in the core policy API so
  daemon, CLI, and diagnostics use the same canonical names.
- Include policy usability explicitly in JSON backend diagnostics instead of
  conflating detected protocol state with a selectable backend.
- Keep backend diagnostic policy names in the core backend contract rather than
  duplicating them in the CLI.
- Make JSON doctor derive policy reporting and backend filtering from one
  policy load, avoiding inconsistent snapshots during policy replacement.
- Reject relative XDG and portal-token paths so daemon file locations cannot
  depend on a working directory.
- Correct backend readiness reporting, CLI help, atomic backups, stable exit
  categories, and the `disable_title_matching` fail-closed contract.
- Ensure `doctor` never labels a non-invasive protocol probe as end-to-end
  `READY`; probe results are reported as available-to-try until live typing is
  certified.
- Share evdev's keyboard-capability probe with setup and doctor so readable
  non-keyboard event nodes cannot be presented as usable capture devices.
- Replace IBus' constant whole-file reload polling with parent-directory
  notifications, retaining a bounded polling fallback when watching is not
  available.
- Require keyboard-layout and target-client metadata in compositor evidence so
  certification records are reproducible rather than merely version-labeled.
- Require doctor JSON readiness to observe a complete capture/output pair;
  output-only protocol globals no longer imply a usable backend.
- Add the doctor JSON readiness and selection fields to the checked-in CLI
  contract fixture so integrations receive an explicit stability guard.
- Source CLI policy diagnostics from the core policy-path constant, removing
  another duplicated trust-boundary value.
- Add CI coverage for certification evidence acceptance and rejection rules,
  including missing scenarios, explicit failures, and missing metadata.
- Correct the doctor JSON example so its automatic-selection contract matches
  the conservative resolver rather than implying setup persisted a daemon
  backend selection.
- Harden CI and release workflows with complete actionlint coverage, pinned
  Syft artifacts, consistent MSRV/version validation, and corrected Launchpad
  synchronization YAML. The actionlint image is digest-pinned, and Launchpad
  synchronization uses the canonical SSH URL and refuses non-fast-forward
  branch or tag overwrites.
- Reconcile compatibility, setup, operations, and policy documentation with
  the currently implemented backend and certification boundaries.

- Add a shared generation-aware `ConfigStore` and live IBus configuration
  reloads, preserving pause, sensitive-focus, window-context, and command
  safety state across replacement.
- Make input-method-v2 opt-in visibly experimental through
  `wayexpand setup --experimental-input-method-v2`; document its exclusive
  capture and unsupported-key limitations alongside evdev and IME/preedit
  limitations.
- Simplify onboarding with `wayexpand setup`, `wayexpand status`, and
  `wayexpand edit`; stop packaging the stdin test harness as
  `wayexpand.service` while retaining expert backend services.
- Pin the CI cargo-audit version, validate GitHub Actions with actionlint, and
  document real KDE, GNOME, Sway, and Hyprland certification requirements.
- Fix portal-token persistence under systemd `ProtectHome=read-only` and make
  the Launchpad sync workflow validate its SSH key and use strict host-key
  handling.
- Forward evdev kernel auto-repeat into matcher state without repeating global
  hotkeys or changing physical held-key state.
- Track native Linux release archives for x86_64 and aarch64 as a packaging goal; only x86_64 is currently published by the release workflow.
- Run daemon command-backed expansions on a bounded background queue. Command
  output is applied only if no intervening input or focus-state change makes
  the original trigger location stale.

## [1.1.2] - 2026-09-18

This is a security and correctness hardening release with 170+ new regression tests covering P0 fixes and stability improvements.

### Added

- `docs/RETRO_FONTS.md`: font recommendations and installation instructions (Ubuntu, Fedora, Arch, openSUSE) for pairing the retro color packs with period-appropriate fonts.

### Fixed

**Security & Correctness (P0):**
- **Cross-window buffer isolation**: Matcher buffer was not cleared when switching windows, allowing text typed in one application to be deleted in another. Buffer now clears on `WindowChanged` event, preventing cross-application interference.
- **Pause/sensitive-field state conflation**: Resume could re-enable expansion capture while still in a password field. Now uses independent `user_paused` and `sensitive_focus` booleans so pause state cannot bypass password-field protection.
- **Ancestor path validation**: Incomplete validation allowed world-writable non-sticky ancestors to permit config path replacement. Now walks complete path to root, validating owner and permissions on every ancestor up to the mount point. (Future: improve with `openat2(RESOLVE_BENEATH)` once fd-based path validation is standardized.)
- **Silent clipboard fallback**: Multiline expansions silently switched from Wayland injection to X11/XWayland, potentially targeting wrong windows. Fallback is now explicit and controlled, never silent.
- **input-method-v2 key loss**: Unsupported keys (Escape, arrows, F-keys) were silently discarded without warning. Documentation now clearly warns this is a known limitation requiring workaround for affected compositors.

**Reliability & Data Loss:**
- **KWin window-tracker D-Bus hang**: `KwinWindowTracker::probe()` called blocking D-Bus in daemon `main()` before event loop with no timeout, causing indefinite hangs with no diagnostic output. Now bounded to 3 seconds on detached thread.
- **GUI "Use current app" hang**: Same unbounded-hang exposure in window-tracker GUI button. Moved to background thread with spinner and Cancel button.
- **Clipboard backend timeouts**: Every spawned process (`xclip`, `xsel`, `xdotool`, `which`) had no timeout, allowing hung X server to stall the entire daemon. Now bounded to 3 seconds per call.
- **Undo discarding edits**: Undo bypassed unsaved-draft confirmation that other destructive actions enforced. Also fixed transactional bug where failed disk write still consumed undo-history and changed state.
- **Window close without save confirmation**: Closing GUI with unsaved draft had no confirmation. Now routed through Save/Discard/Cancel dialog.
- **Command-backed expansions in preview**: GUI's live snippet preview executed command-backed expansions on every repaint, running unconfigured programs. Preview now never auto-executes commands; explicit "Run once" button does with cached result.

**Correctness:**
- **Daemon reload after edit**: Create, Duplicate, Delete, and Undo saved to disk but never reloaded the daemon, allowing deleted snippets to still expand. Now surfaces reload request status in UI.
- **propagate_case validation**: Case propagation could silently collide with unrelated snippets. Validation now checks effective triggers, not just literal configured ones.
- **xdotool exit status**: Clipboard backend ignored `xdotool` failures, allowing failed erases to leave the trigger typed after the replacement. Now checks exit status.
- **Pause/Resume state**: GUI's Pause/Resume button assumed daemon was running on startup. Now queries actual daemon state first.
- **Clipboard fallback detection**: `xsel` fallback was skipped when `xclip` binary was entirely absent. Now properly handles `xsel`-only installs.
- **Clipboard X11 requirement**: Clipboard backend now refuses to initialize when no `DISPLAY` is available, avoiding silent failure on every operation.

**Stability:**
- **Mutex poisoning in window-tracker**: D-Bus callback could panic on every window-focus event, permanently breaking `app_filter` until restart. Now handles without panicking.
- **TUI panic recovery**: `wayexpand-ui` panic during main loop (running in raw/alternate-screen mode) left terminal stuck. Now restored via RAII guard that runs even during panic.

**Documentation & Packaging:**
- Default theme's muted-text and border colors brightened for WCAG AA contrast compliance.
- Application icons now included in all install paths (PKGBUILD, RPM spec, `install-user.sh`, `install-release.sh`).
- `docs/COMPATIBILITY.md` corrected: `status --json` section documented non-existent fields. Now documents actual fields with contract tests to prevent future drift.
- README.md and README.de.md rewritten with architecture diagram and accurate backend compatibility claims.
- Resolved GNOME window-tracking contradictions between roadmaps and docs.
- Release workflow now verifies `debian/changelog`, `PKGBUILD`, and `wayexpand.spec` versions against git tag, preventing stale package metadata.

## [1.1.1] - 2026-09-18

### Added

- `docs/SYSADMIN_EXAMPLES.md`: 30+ production-ready snippet templates (SSL certificates, logrotate, systemd units, firewall rules, Docker, deployment scripts).

### Fixed

- Color contrast in the Classic White, Terminal Blue, and Commodore 64 themes, which were hard to read against their backgrounds.

## [1.1.0] - 2026-09-18

### Added

- **GUI language support**: English and German, with in-app switching (🌐 button), `LANG` environment auto-detection, and persisted preference. See [docs/LANGUAGE_SUPPORT.md](docs/LANGUAGE_SUPPORT.md).
- **GUI color packs**: eight selectable themes, including retro monochrome terminal styles (Classic Green, Classic Amber, Classic White), Retro 80s Neon, a High Contrast accessibility theme, and two new additions this release — Terminal Blue (IBM 3270) and Commodore 64 — alongside the Default theme. Preference persists across restarts. See [docs/COLOR_PACKS.md](docs/COLOR_PACKS.md).
- **GUI font scaling**: 0.8x-2.0x, for accessibility and high-DPI displays, with a 5-option selector in Settings.
- **Keyboard focus indicators, typography hierarchy, and hover-state polish** across the GUI.
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
- Control socket and daemon security model formally documented in [SECURITY.md](SECURITY.md)

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

[Unreleased]: https://github.com/itchyitchy123/wayexpand/compare/v1.1.2...HEAD
[1.1.2]: https://github.com/itchyitchy123/wayexpand/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/itchyitchy123/wayexpand/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/itchyitchy123/wayexpand/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/itchyitchy123/wayexpand/releases/tag/v1.0.0
[0.2.0]: https://github.com/itchyitchy123/wayexpand/releases/tag/v0.2.0
[0.1.0]: https://github.com/itchyitchy123/wayexpand/releases/tag/v0.1.0
