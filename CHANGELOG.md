# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

Changes not yet released.

- UX: the active injector now publishes its insertion mode, character limit,
  and paced throughput estimate; the GUI warns when a snippet exceeds that
  live backend limit.

- UX: onboarding now leads with user-facing integration properties and keeps
  capture/output backend names in the expanded technical details.

- Fix: app-ID glob filters now match `?` against Unicode scalar values instead
  of UTF-8 bytes, so international application IDs are matched correctly.

- Fix: evdev now replays keys held on surviving keyboards when rebuilding its
  XKB state after another keyboard disconnects, keeping modifier handling and
  per-device pressed-key tracking consistent.

- Fix: paced libei keysym fallback output no longer blocks the daemon's
  non-exclusive evdev capture loop; replacements are serialized through a
  bounded output worker and worker failures trigger the normal reconnect path.

- **Breaking — app filters are exact by default:** a bare `app_filter` value
  now matches one normalized app ID exactly instead of any app ID containing
  it, so `["thunderbird"]` no longer matches `org.mozilla.thunderbird`. Use
  `app_id_exact:`, `app_id_glob:`, or `title_contains:` explicitly; safe mode
  rejects the weak forms unless `allow_weak_app_filters = true`. Snippets with
  old substring filters stop expanding (fail closed) until updated; see
  `docs/UPGRADING.md`.
- **Matcher preview:** a panel in the GUI expands your saved snippets as you
  type with the daemon's matching engine, with no setup or permissions, and
  counts the keystrokes saved. It does not exercise desktop capture or
  insertion.
- **Quick-insert picker:** `wayexpand-gui --picker` (also a *Quick Insert
  Snippet* desktop action) searches your snippets from a shortcut and has the
  running daemon type the chosen one into the previously focused app once
  focus verifiably returns to it, which needs focused-window tracking
  (currently KWin); otherwise it copies to the clipboard. `wayexpand insert <trigger>` and the
  `insert` control command do the same for scripts, under the typed-expansion
  safety rules.
- **One-click setup and a three-step first run:** *Turn on WayExpand* runs the
  safe Recommended setup from the GUI.
- Fix: the daemon, IBus engine, and action broker logged only errors unless
  `RUST_LOG` was set, so warnings such as policy violations never reached the
  journal; they now default to `info`, and no longer write colour codes to
  non-terminal output.
- Fix: the GUI showed a running daemon without a compositor session (stdin
  source) as disconnected.
- README and AppStream metadata now include real screenshots.
- input-method-v2 capture now refuses to start without libei key
  pass-through, so an exclusive keyboard grab can never swallow keys it cannot
  forward; IBus never runs command-backed snippets, in safe or audit mode.
- Injection failures distinguish "not applied" from "possibly partially
  applied": only the former keeps an undo record for retry. A lost libei or
  wlroots session still reconnects. A failed `{{cursor}}` move after a
  successful replacement is logged instead of failing the expansion.
- Snippet packs are read as hostile input: size, file-count, and aggregate
  limits, and symlinks or special files are rejected.
- Configuration writers give up with a "busy" error after two seconds instead
  of waiting forever on another writer's lock.
- libei's per-key fallback refuses replacements over 250 characters (about
  three seconds of synthetic typing) before erasing the trigger.
- GUI saves run in the background; a save that completes while the snippet is
  still being edited keeps the newer edits in the editor.

- TUI: accept plain comma-separated tags again (`ops, email`); a JSON array is
  still accepted for a tag containing a comma. The JSON-only prompt introduced
  in the previous change also broke `scripts/test-ui.sh`.
- TUI: hand replacements to `$VISUAL`/`$EDITOR` in the private
  `$XDG_RUNTIME_DIR` instead of the shared `/tmp`, and stop repainting the
  whole screen four times a second while idle.
- CLI: errors keep their cause (`creating configuration backup …: File
  exists` instead of only the first half), and repeated `wayexpand backup`
  runs create `.bak.2`, `.bak.3`, … instead of failing.
- Fleet and administrator-policy errors in the CLI and daemon reload log no
  longer echo trigger text, and the daemon reports fleet conflicts as such
  instead of "configuration could not be read consistently".
- GUI: the snippet list lays out only the visible rows, so very large
  libraries stay responsive.

- GUI: fix washed-out, disabled-looking text in every field and button (the
  theme painted idle widget text with the border colour); fix the light-theme
  status bar rendering near-black with unreadable text; stop the snippet
  editor overflowing the window and overlapping rows in narrow windows.
- GUI: the unsaved-changes and delete confirmations are now true modals, so
  clicking elsewhere can no longer replace the pending action, and Escape
  cancels them; Ctrl+S now saves a new snippet and no longer rewrites the file
  for an unchanged one.
- GUI: tags and app filters are edited as chips; template variables insert at
  the caret; Save, Duplicate, and Undo are disabled when they would do nothing;
  the toolbar is a single row; command-backed snippets show their own
  description in the list; new snippets no longer get a "New snippet"
  description; the status bar shows paths relative to `~`.
- GUI: translate the remaining English-only editor strings (tag, app, argument
  and environment controls, template variable descriptions) into German, and
  correct the command-backed note, which wrongly called the replacement a
  fallback.
- Accept `{{ cursor }}` with inner whitespace like every other template
  variable, and report a second `{{cursor}}` marker as its own error instead
  of "unknown template variable". Fix a documented release-tag example that
  used three markers and failed validation.
- Report configuration parse errors with their line and column (and the name
  of a missing required field) while still never echoing snippet content.
- CLI: a missing configuration file now names the path and how to create one;
  `wayexpand <command> --help` prints help instead of treating `--help` as an
  argument; invalid `set-enabled`/`set-mode` values exit with the documented
  usage code 2; the help screen is grouped and aligned and lists every command.
- Fix `wayexpand --version` reporting the previous commit in development
  builds: the build script now tracks the checked-out branch ref, not only
  `.git/HEAD`.
- Add the required `replacement = ""` to command-backed snippet examples in the
  Espanso migration, Ansible, and security documentation.

- Add typed injector capability contracts and organization requirements for
  atomic replacement and sensitive-field awareness. Safe-mode deployments now
  fail before capture starts when their selected source/backend cannot meet the
  requested guarantees; input-method-v2 is the current path satisfying both.
- Require a separate `--allow-evdev-sensitive-fields` acknowledgement before
  starting evdev capture. Raw keyboard permission alone is not a sufficient
  acknowledgement of evdev's inability to detect password fields; the shipped
  evdev service declares the risk explicitly.
- Protect deferred command reservations when an asynchronous command fails, so
  a failed or stale completion cannot leave its trigger missing from the
  matcher state.
- Run expansion commands through a bounded four-worker pool; one slow command
  no longer serializes unrelated expansion commands while hotkeys retain their
  separate worker and queue.
- Refuse to overwrite a configuration that changed outside the GUI since it
  was loaded or last saved; the GUI now reports the conflict and preserves the
  external edit until the user explicitly reloads it.
- Prevent repeated cancellation of GUI application detection from leaking
  detached D-Bus worker threads, and discard late detection results after the
  user switches snippets or cancels the request.
- Polish the GUI's shared visual language: controls now have consistent sizing,
  quieter hover states, visible keyboard focus rings, correctly composited
  status pills, and more deliberate spacing across the toolbar, library, and
  editor surfaces.
- Fix the daemon reporting an incomplete, partly fabricated status while the
  input-method source is reconnecting: that one transition formatted its own
  status line, omitting `backend_mode` and all nine command-metric fields from
  the documented control-socket contract and asserting `paused=false` and
  `config_state=ok` whatever the real state was. `ControlServer::set_status`
  now accepts only a `StatusBody` built by the shared writer, so the contract
  test that covered the builder can no longer be bypassed.
- Remove per-keystroke allocations from the expansion hot path. Continuation
  checks read the matched suffix straight out of the rolling buffer instead of
  collecting it into a `String`, and the byte budget measures the trigger the
  match plan already carries instead of cloning it out of the configuration.
- Stop deep-copying the match plan on every expansion. The preflight policy
  check borrowed nothing and returned the plan it was handed, so every match
  copied three strings and the command `Arc` only to drop the copy; the
  rendered replacement and the trigger now move into the result rather than
  being cloned beside it.
- Stop rebuilding a `MatchPlan` from the configuration for each completed
  asynchronous command. The postflight check reads only the input generation
  and the output, so the trigger, replacement, and command copies it forced
  are gone.
- Apply an expansion without copying the matched text and the replacement when
  no terminating character has to be carried through, and trim command output
  in place instead of copying up to a megabyte to drop a trailing newline.
- Unify the immediate and deferred event processors, which carried two copies
  of the same matching policy — a fix applied to one could silently miss the
  other, and the daemon runs the deferred one. Buffer maintenance, undo
  invalidation, the state-only events, and both match decisions are now shared,
  with a test asserting the two paths agree on what matches.
- Avoid re-filtering the whole GUI snippet library twice per frame, and search
  each field in turn instead of building a joined, lowercased copy of every
  snippet's searchable text on every frame.
- Compare the GUI's comma-separated tag and app-filter fields in place rather
  than rebuilding the joined strings on each of the several dirty checks per
  frame, and only parse the draft command once every cheaper field has matched.
- Remove two empty crate directories (`backend-clipboard`,
  `backend-wlroots-toplevel`) that no longer held any source and were not
  workspace members, and drop the two write-only snapshot fields from
  `MatchPlan`.
- Consolidate GUI preferences into one tabbed `Settings` window: an Appearance
  tab (theme, language, font scale, color pack) that applies and persists on
  click, and a Typing engine tab (buffer limit, undo chord) committed by an
  explicit Save. The separate `Language` and `Color pack` toolbar buttons and
  their standalone windows are gone; a light/dark toggle remains in the
  toolbar. Font scale no longer consumes an undo step.
- Give the GUI a pinned status line that reports the outcome of the last
  action and the configuration file in use. The outcome is now recorded
  explicitly by whatever performed the action instead of being guessed from
  English keywords in the rendered sentence, which silently mis-coloured every
  German message and any reworded English one. A failed daemon reload after a
  successful save now downgrades the line to a warning rather than reading as
  an unqualified success.
- Show the open configuration file, with an unsaved-changes marker, in the GUI
  window title, and pin `Save changes`/`Delete` to a bar below the editor so
  the primary action no longer scrolls out of reach behind a long replacement.
- Lay the GUI snippet form out on an aligned two-column grid, so fields no
  longer start at a different horizontal position depending on how long each
  label happens to be in the selected language.
- Translate the remaining hardcoded English in the GUI — status messages,
  settings and appearance chrome, command preview, window detection, sidebar
  placeholders, and diagnostics — so a German session is no longer half
  English. Backend states are shown as readable labels next to the verbatim
  `doctor` triple rather than as raw Rust debug output.
- Add GUI keyboard handling for `Ctrl+F` (focus search) and make `Esc` close
  the topmost dialog; editor accelerators are now suppressed behind every
  dialog, not just three of them.
- Fix GUI details that clipped or rendered twice: the category picker drew two
  dropdown arrows, and the search and app-filter placeholders were cut off
  mid-word.
- Improve GUI font coverage with validated platform fallbacks for symbols and
  CJK text, and use the shared card/secondary-control treatment in diagnostics.
- Fix Action Broker IPC parsing for valid UTF-8 frames whose characters are
  split across socket reads.
- Polish the GUI with a layered two-tier toolbar, consistent secondary
  controls, color-pack preview cards, font-scale-aware custom widgets, and
  responsive explicit command previews.
- Refresh GUI and broker documentation with a current screenshot, corrected
  implementation claims, and a CI documentation-contract check that catches
  version, color-pack, screenshot, service, and removed-setting drift.

- Refuse to prepare a release while an unprefixed tag with the same version
  exists, keeping public release references aligned with the `vX.Y.Z` workflow.

- Require release tarball generation to use the locked Cargo dependency graph
  when vendoring dependencies.

- Make Debian builds fail clearly when the required vendored source archive is
  missing instead of retrying unavailable crates.io network access.

- Make Launchpad synchronization use its decoded SSH key explicitly in the
  push step; GitHub Actions does not preserve an ssh-agent environment between
  steps.
- Harden the experimental Action Broker by validating and canonicalizing
  executable targets at startup and bounding concurrent action execution;
  distribution installers continue to omit the broker until routing and
  managed service deployment are implemented.
- Require Action Broker working directories to be absolute, private, existing
  directories and canonicalize them during policy loading.
- Validate canonical executable and working-directory ancestors so writable path
  components cannot replace or redirect broker targets after policy loading.
- Reconcile the professional roadmap with the Action Broker’s implemented
  foundation and keep sandboxing, audit, routing, and service certification
  explicitly open.
- Correct operational documentation to use the shipped sandboxed user-unit
  names and remove examples for nonexistent or unsandboxed daemon services.
- Add a non-secret Launchpad SSH preflight that reports the decoded key
  fingerprint and distinguishes authentication failure from push failure.
- Accept both SSH client exit statuses used for Launchpad's authenticated
  `No shells on this server.` response.
- Treat Launchpad's authenticated no-shell response as the stable SSH
  preflight signal regardless of the runner's transport exit status.
- Make Launchpad tag synchronization idempotent and fail closed on divergent
  existing tags instead of force-overwriting release history.
- Allow Launchpad synchronization to be rerun manually after an administrator
  resolves remote release-tag state.
- Make the headless capture-readiness regression test accept the documented
  `not-probed` state used by CI runners without a graphical session.
- Serialize CI test harnesses to avoid hosted-runner races in tests that
  exercise bounded worker resources and process-wide desktop state.
- Remove process-wide `HOME` mutation from the GUI import-path test so CI
  tests remain isolated under parallel execution and older Rust toolchains.
- Ensure explicit installer service enablement restarts the selected daemon,
  and warn when an ordinary upgrade leaves an existing service process running.
- Clean up a partially started async command worker when the hotkey worker
  cannot spawn, preventing thread leaks during bounded-resource fallback.
- Align human doctor output with JSON by labeling heuristic libei-only paths
  `AUTHORIZATION REQUIRED` instead of implying they are already available.
- Run the compositor certification matrix weekly as well as on manual
  dispatch; runner-provided GTK/Qt drivers remain mandatory and fail closed.
- Make explicit input-method-v2 selection fail closed when its live protocol
  probe is unavailable, keeping daemon startup aligned with shared capability
  diagnostics.
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
- Bind the self-hosted certification job to the exact debug binaries it builds
  before collecting doctor and evidence probes.
- Protect the four-desktop certification workflow with a matrix contract test
  that checks its driver, evidence, and runner-metadata requirements.
- Align human doctor capture readiness with JSON diagnostics when IBus is the
  only available, policy-allowed input path.
- Align human doctor exit status with JSON health: authorization-only, headless,
  and otherwise unverified paths remain visible but are not reported healthy.
- Clarify that `AVAILABLE TO TRY` is not a successful doctor result and does not
  authorize starting a backend service without reviewing its requirements.
- Quote CI workspace paths so actionlint's ShellCheck pass protects the workflow
  itself instead of failing on avoidable word-splitting warnings.
- Preserve per-scenario compositor-driver logs in certification artifacts so a
  passing result remains reviewable evidence rather than only an exit status.
- Keep human and JSON doctor health consistent when no graphical display session
  is active, including when the IBus component is installed.
- Scope release workflow write, OIDC, and attestation permissions to the
  publishing job; reusable verification CI remains read-only.
- Update the CI dependency audit tool to cargo-audit 0.22.2, which parses the
  current RustSec advisory database including CVSS 4.0 records.
- Align release documentation with the pinned release compiler and the
  AppStream/IBus metadata checks enforced by the workflow.
- Mark the daemon’s stdin-only fallback as unsupported in certification reports
  instead of presenting it as an available automatic keyboard path.
- Unify human and JSON doctor control-socket validation, including socket type,
  ownership, parent-directory trust, permissions, and missing configured sockets.
- Share the IBus installation probe with CLI and GUI diagnostics, and stop the
  GUI from coloring merely implemented-but-unverified backends as ready.
- Show shared organization-policy validity in GUI diagnostics so policy failures
  cannot be hidden behind backend availability rows.
- Bound IBus registry probing and prefer the installed component manifest so a
  broken IBus daemon cannot freeze GUI or CLI diagnostics.
- Update doctor’s configuration test to distinguish valid configuration from
  unavailable desktop backends.
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

## [1.2.0] - 2026-09-23

### Backend & Keyboard Correctness
- Configure libei token storage for input-method service with secure systemd hardening
- Add CI contract test to verify all libei-using units have proper token persistence
- Make libei key pass-through optional and non-blocking during input-method startup
- Implement automatic graceful degradation when libei is unavailable
- Fix child process cleanup to work on all error paths (OutputTooLarge, read errors, fd setup)
- Add RAII guard to ensure process reaping regardless of return path
- Add regression test verifying child process killed when output exceeds 1MiB limit

### Quality & Testing
- Add comprehensive pass-through contract tests (9 new regression tests)
- Implement bounded pending_key_pass_through queue (MAX_PENDING_KEY_PASS_THROUGH=512)
- Add queue overflow detection with retryable error reporting
- Derive Debug and PartialEq for KeyAction to enable test assertions

### User Experience
- Parse EDITOR and VISUAL environment variables safely using shlex
- Fix broken external editor integration for commands with arguments (e.g., `code --wait`)
- Handle both single-word and multi-argument editor specifications

### Documentation
- Resolve contradictions in key pass-through capability documentation
- Make conservative language authoritative: document as experimental pending certification
- Update SUPPORT_MATRIX.md with explicit validation gaps
- Update AUDIT_FINDINGS.md to clarify certification requirements
- Update source code module docs and user-facing messages for consistency

### Release Infrastructure
- Reorder release workflow to generate SBOMs from actual artifacts, not CI workspace
- Extract source tarballs before scanning for accurate supply chain attestation
- Fix vendor SBOM generation to scan extracted tarball, not CI workspace vendor/
- Improve supply chain story through accurate Software Bill of Materials

### Deprecated
- Delete v1.2 tag (non-SemVer, divergent from main branch history)

## [1.1.2] - 2026-09-18

This is a security and correctness hardening release with 170+ new regression tests covering P0 fixes and stability improvements.

### Added

- `docs/GUI.md`: font recommendations and installation instructions (Ubuntu, Fedora, Arch, openSUSE) for pairing the retro color packs with period-appropriate fonts.

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

- **GUI language support**: English and German, with in-app switching (🌐 button), `LANG` environment auto-detection, and persisted preference. See [docs/GUI.md](docs/GUI.md).
- **GUI color packs**: eight selectable themes, including retro monochrome terminal styles (Classic Green, Classic Amber, Classic White), Retro 80s Neon, a High Contrast accessibility theme, and two new additions this release — Terminal Blue (IBM 3270) and Commodore 64 — alongside the Default theme. Preference persists across restarts. See [docs/GUI.md](docs/GUI.md).
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

[Unreleased]: https://github.com/cyberducttape/wayexpand/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/cyberducttape/wayexpand/compare/v1.1.2...v1.2.0
[1.1.2]: https://github.com/cyberducttape/wayexpand/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/cyberducttape/wayexpand/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/cyberducttape/wayexpand/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v1.0.0
[0.2.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v0.2.0
[0.1.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v0.1.0
