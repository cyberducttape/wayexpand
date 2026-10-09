# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

- Added `wayexpand support-bundle`, an allowlisted diagnostic summary that
  excludes configuration paths, snippet contents, credentials, and raw errors;
  optional output files are private and non-overwriting, while diagnostic
  subprocesses have bounded output and teardown.
- User installer version directories now use the built binary's full version
  identity, so installs from newer development commits cannot reuse stale
  files merely because the Cargo package version is unchanged.
- Systemd unit CI now creates dummy executables at the versioned installer's
  active `current/bin` path, keeping service validation aligned with installs.
- GUI command previews now distinguish local direct execution (outside both
  service sandboxes and broker audit) from named actions executed through the
  broker, and clarify that broker auditing is optional and health-dependent.
- Managed-action previews no longer apply the direct executable absolute-path
  policy to their empty GUI-side program field; named-action policy remains
  enforced by the broker, while organization command-disable policy still
  blocks both modes.
- User installers now stage all six executables under a versioned library
  directory and atomically switch the `current` pointer; enabled upgrades
  preflight systemd availability, verify service activity, and restore the
  prior pointer if reload, enablement, or startup fails.
- Installer `--enable`/`--service` validation now runs before build or file
  changes, and both installers reject incomplete service selections early.
- Secure config saves now retain no-symlink directory traversal when `openat2`
  is unavailable or blocked, using descriptor-relative `openat` with
  `O_NOFOLLOW` rather than an unsafe path-based fallback.
- Git synchronization now tracks the configured primary filename, rejects a
  symlinked primary config, and verifies outgoing tree modes as well as paths.
- Usage-stat persistence now runs on a dedicated bounded-queue worker rather
  than the input reactor, with failure/drop counters and maximum flush duration
  exposed in daemon status.
- Redacted credentials in Git synchronization status and diagnostics, and use
  the configured `origin` name for fetch/push operations instead of passing
  credential-bearing URLs through those commands.
- Generated user-service systemd drop-ins from the installer's resolved XDG
  paths so daemon token access, sandbox write permissions, and broker policy
  configuration stay aligned for custom configuration and state directories.
- Added regression coverage for atomic, private Action Broker health-file publication.
- Prevented GUI Git-sync timeouts from hanging on blocking output-reader joins.
- Installer upgrades now validate existing libraries before replacing installed files.
- Bounded pack-signature subprocess input and output teardown after timeout or inherited pipes.
- Bounded GUI setup subprocess teardown and removed its PID-reuse process-group race.
- Guided setup now fails closed if its input-method route catalog is inconsistent.
- Control-socket teardown now wakes and joins its listener thread instead of
  leaving an accept loop alive after the server is dropped.
- Window-tracker teardown now joins its reconnect supervisor instead of
  leaving a background tracker thread detached.
- Fleet reloads now verify that all layer files remain stable across the merge,
  preventing a mixed snippet snapshot from becoming active during an edit.
- Form-helper failures now stop their transient systemd unit, including
  inherited-pipe and malformed-output paths, preventing orphaned GUI helpers.
- Git synchronization now serializes the complete repository transaction,
  preventing concurrent callers from interleaving fetch, rebase, and rollback.
- Runtime diagnostics now distinguish a connected window tracker from an exact
  focused-window identity, preventing false app-filter readiness reports.
- Daemon startup now preserves an existing control socket when its ownership
  check fails for a non-stale error, instead of unlinking a potentially active
  endpoint.
- Synchronization and CLI validation now merge the portable base config with
  `snippets.d`, catching duplicate triggers and hotkeys before daemon reload.
- Daemon fleet composition now applies the same stable-ID update semantics when
  layering fleet snippets over the primary configuration.
- Fleet status statistics now count the primary configuration file even when it
  contains only settings, keeping loaded-file diagnostics accurate.
- Preserved the latest hosted engine-sequence fuzz input as a stable regression
  case so future engine changes replay the reported state sequence.
- Added concurrent Action Broker startup coverage to ensure only one process can
  own a socket and failed starters cannot replace the active endpoint.

- **Policy-safe GUI command previews:** explicit command previews now honor the
  effective organization policy before spawning a local program, preventing
  managed `safe_mode` deployments from bypassing command restrictions.
- **Release certification secret forwarding:** release builds now inherit the
  configured compositor-runner token when calling the reusable certification
  workflow, so tag releases can reach the runner-capacity preflight.
- **Bounded Launchpad synchronization:** the mirror job now has a workflow-level
  timeout in addition to its bounded SSH and push operations.
- **Fail-closed manual uninstall:** uninstall now detects running WayExpand
  processes even when the user systemd bus is unavailable, preventing active
  input capture or injection from surviving removal.
- **Source-independent evdev cleanup:** revoking raw-input permissions no
  longer requires the original source archive or package rule files to exist.
- **Crash-safe evdev state:** permission ownership state is now written to a
  temporary file and atomically renamed, preventing truncated cleanup records.
- **Immediate evdev revocation:** uninstall now triggers existing input devices
  after removing udev rules, so active-seat ACLs are reevaluated immediately.
- **Non-duplicating audit retries:** failed Action Broker audit batches now
  roll back partial appends before retrying, preserving one record per action.
- **Bounded audit shutdown:** broker teardown no longer blocks waiting for a
  flush sentinel when the audit queue is full; queued events still drain after
  the sender closes.
- **Private audit-file creation:** the broker requests mode `0600` while
  creating audit files, closing the permissive-umask exposure window.
- **Exclusive broker health publication:** health snapshots now use exclusive
  temporary files, preventing same-UID symlink races during atomic updates.
- **Reliable IBus setup probe:** the registry probe reads `ibus list-engine`
  output to EOF after the helper exits, so the engine line is not missed.
- **Broker responsiveness with mandatory audit:** waiting for a required audit
  event's fsync no longer blocks an async runtime worker.
- **Cheaper latency diagnostics:** status is rebuilt on every key event, so
  latency percentiles now refresh at most once a second instead of re-sorting
  the sample window each time; counts stay live. The GUI shows "no samples"
  for an empty window and recognizes the Fcitx5 direct-commit output mode.
- **Consistent snippet-layer validation:** the daemon no longer re-validates
  `snippets.d` outside fleet mode (where it never loads those files), and
  `wayexpand validate` follows layer symlinks like the fleet loader, so
  stow-style setups load again. `wayexpand sync` still requires plain files,
  and now fails closed if git's output did not complete.
- **Bounded busy-request drain:** the control socket's overload path discards
  at most 16 KiB of an unread request, so a client that keeps writing cannot
  hold the accept loop.
- **Rejected expansions no longer stop the evdev output worker:** a refusal such
  as a Fcitx5 password-field or trigger-mismatch response (or unrepresentable
  keysym text) now drops only that expansion. Previously it stopped the
  serialized output worker and the daemon exited with "output backend failed
  permanently". Fcitx5 bridge transport failures are now retryable, so the
  route reconnects and re-probes the bridge.
- **Safer Launchpad synchronization:** sync runs are serialized, and version-tag
  checkouts can no longer overwrite the Launchpad `main` branch.
- Launchpad vendored-branch publication now uses an explicit remote lease and
  refuses to overwrite a concurrent branch update.
- Launchpad mirroring retries bounded transient SSH push failures while
  preserving lease and divergence checks.
- Launchpad SSH authorization probes now have explicit connection and overall
  timeouts, preventing an unreachable service from hanging synchronization.
- Compositor certification now performs a hosted runner-capacity preflight and
  fails fast with the missing runner labels instead of remaining queued; the
  preflight uses the explicitly configured Administration-read runner token.
- **Fail-closed certification and rollback:** production input-method
  certification now requires a healthy doctor result, and uninstall refuses to
  remove binaries while any managed user service cannot be stopped or remains
  active.
- **Robust daemon overload handling:** busy control-socket responses are now
  recognized from their exact bounded payload even when the peer close produces
  different platform-specific transport errors.
- **Diagnostics clarity:** the GUI now highlights the negotiated output mode,
  warns when libei keysym fallback is active, separates matcher/preparation
  latency from output-backend latency, and visibly labels organization policy
  enforcement as audit-only when `safe_mode` is disabled.
- **Latency evidence:** daemon status and `status --json` now expose bounded
  p50/p95/p99 injection profiles by negotiated output mode and replacement-size
  bucket, so backend pacing can be evaluated without conflating it with
  matcher latency.
- **Audit and sync privacy:** managed Action Broker deployments can require
  audit confirmation before returning successful action results; failed audit
  writes are retried and surfaced as action errors. Git sync commit messages
  no longer disclose the workstation hostname.
- **Bounded Git supervision:** synchronization now drains Git output without
  blocking reader-thread joins when descendants retain inherited descriptors.
- **Command-worker scheduling:** expansion workers now share a bounded
  condition-variable queue that releases its lock while idle, avoiding
  serialized receiver timeouts without changing queue backpressure.
- **Bounded helper supervision:** action-broker output draining now yields to
  action deadlines, form cancellation terminates process groups and transient
  systemd units without an unbounded reader join, and IBus discovery drains
  registry output while monitoring the child process.
- **GUI polish:** localized the command editor's execution/action controls in
  German mode and added descriptive accessibility metadata to its move/remove
  buttons, so icon-only controls are understandable to assistive technology.
- **Project polish:** refreshed the public README with a concise status table
  and safer first-run commands, added repository-wide editor conventions,
  default code ownership, and privacy-aware feature/support issue forms.
- **Optional Fcitx5 integration:** the libei route can now use a compatible,
  exact-surrounding-text Fcitx5 bridge when explicitly enabled. Missing bridges
  preserve existing behavior; sensitive, mismatched, unknown, and uncertain
  bridge outcomes fail closed before any raw erase. Unicode replacements now
  reach the bridge even when the negotiated EIS device only provides the
  layout-dependent keysym fallback; the paced fallback limits still apply when
  the bridge declines because no Fcitx context is focused.
- **Authoring interoperability:** `wayexpand schema` exposes a versioned,
  machine-readable TOML configuration shape for editors and integrations while
  leaving semantic validation in the shared `validate` path.
- **Synchronization integrity:** `wayexpand sync` and daemon reloads now share
  full-library validation for `snippets.d/*.toml`, validate rebased remote
  content before promotion, reliably stage deletion of the final snippet, and
  reject malformed compatibility versions in pack manifests.
- **Broker singleton safety:** a second same-user action broker now refuses to
  start while the existing Unix socket is live; stale owned sockets remain
  recoverable without unlinking an active endpoint.
- Cache the editor preview's library snippet lookup by configuration revision, avoiding a full-library scan and clone after each edit while keeping dynamic template values fresh.
- Preserve production `{{snippet:...}}` include validation in single-snippet editor previews without rebuilding the full trigger matcher.
- **Production-readiness architecture:** split the Action Broker protocol and
  client from its server/executor implementation; the packaged broker is
  explicitly local-action-only because its systemd sandbox denies networking.
  Frontends now route daemon controls through typed shared operations, and
  daemon route status is parsed centrally by `wayexpand-core`.
- **Structured diagnostics:** command failures expose stable error categories
  for GUI, TUI, and doctor remediation without requiring display-string
  parsing.
- **Fail-closed daemon status:** ambiguous duplicate route-state fields are
  rejected instead of allowing a conflicting status response to appear
  connected or healthy.
- **Safer setup and maintainability:** experimental setup refuses unavailable
  backend routes; engine, backend-selection, daemon socket, fleet discovery,
  GUI localization, IBus installation discovery, text/action helpers, and
  policy enforcement were split along clearer boundaries.
- **CI and certification integrity:** concurrent stress coverage now includes
  evdev, wlroots, backend selection, the CLI, and process supervision. Release
  checks continue to require durable compositor evidence and do not claim
  production certification without it.
- **Failure containment:** secure libei portal-token persistence now reports a
  missing temporary file as an I/O error and cleans up instead of panicking in
  the desktop integration path.
- **Bounded discovery:** the IBus registry availability probe caps output from
  `ibus list-engine` before inspecting it, preventing an untrusted or broken
  helper from causing unbounded allocation during setup checks.
- **Pack-signing containment:** `ssh-keygen` used by pack signing and
  verification now has a bounded output budget, a deadline, and process-group
  cleanup on timeout or monitoring failure.
- **Sync containment:** Git operations used by `wayexpand sync` now have a
  bounded output budget, a 60-second deadline, and process-group cleanup so
  remote operations cannot hang the CLI indefinitely.
- **GUI setup cleanup:** cancelling or timing out the graphical setup flow now
  terminates the complete CLI process group, including any system setup helper
  descendants.
- **CLI setup containment:** direct `wayexpand setup` IBus and systemd helper
  calls now have 30-second deadlines and process-group cleanup as well.
- **Bounded broker diagnostics:** `wayexpand doctor` now uses the same deadline
  and process-group cleanup when checking the broker's systemd service.
- **GUI setup pipe cleanup:** when the setup CLI leader exits, inherited helper
  processes are terminated before GUI output collection, preventing a lingering
  descendant from keeping the worker blocked on `wait_with_output`.
- **Tracker shutdown:** the KWin window-tracker reconnect backoff is now
  interruptible, so daemon shutdown does not wait for the full 30-second retry
  interval.
- **GUI sync containment:** Library → Sync now bounds its child process,
  output capture, deadline, and process-group cleanup instead of bypassing the
  CLI Git safety controls.
- **Output worker retirement:** a timed-out serialized output worker is retired
  without joining it, and output recovery waits until that worker has stopped
  before reconnecting, so an uncertain operation never overlaps a new route.
- **Bounded reloads and control overload:** configuration reloads verify a
  stable source revision across atomic renames with bounded retries, reload
  notifications are coalesced, and an overloaded control socket answers
  `error=busy` (counted separately from the keyboard data plane).
- **Backend safety diagnostics:** the GUI diagnostics page states that checks
  verify protocol availability, not live typing, and warns when a route lacks
  sensitive-field protection or atomic output.
- **Bounded command stdout:** a command whose stdout is continuously readable
  can no longer starve its own deadline or cancellation checks.
- **Bounded fleet integrity scans:** the periodic fleet signature reads each
  layer through the configuration size limit.
- **Control busy responses delivered:** the daemon now drains fragmented and
  oversized requests before replying `error=busy`; previously Linux could
  report a connection reset and clients saw a generic read error instead of
  the typed `Busy` error.
- **Sync commit isolation (security):** `wayexpand sync` commits only the exact
  allowlisted library paths, leaving unrelated staged files untouched, and
  refuses to push when outgoing history touches anything outside the library.
- **Form cursor safety:** snippet forms are opened, and their results applied,
  only on output routes that verify the trigger is still at the cursor
  (`verified` or `atomic`). On best-effort routes (evdev with libei or wlroots)
  form snippets are refused and the trigger is left in place, since a click
  could have moved the caret while the form was open.
- **Evdev delimiter folding:** delimiters absorbed during the evdev quiet
  period keep a `{{cursor}}` marker at its intended position, and results with
  several folded delimiters release their deferred-match reservation instead
  of reviving an already-applied trigger.
- **Evdev output recovery:** a retryable injection failure on the evdev path
  (or while applying completed commands) now retires the output route and
  reconnects instead of ending the daemon's event loop.
- **Special-file safety in fleet scans:** fleet discovery ignores FIFOs,
  devices, and directories named `*.toml`, and integrity probes never open a
  non-regular file, so a FIFO can no longer block the keyboard loop.
- **Sync remote verification (security):** sync fetches and refuses a remote
  tree containing symlinks, submodules, or files outside the library before
  checking anything out, and permission repair never follows symlinks.

Changes not yet released.

- **GUI undo while saving:** pressing Undo again before the previous save
  finished could crash the editor or silently discard an in-flight edit.
  Undo is now disabled until pending saves complete.

- **Hotkey process cleanup:** hotkey actions now kill their process group
  before reaping the leader, like expansion commands, so a recycled PID can
  never receive the cleanup signal.

- **Action Broker:** actions no longer inherit the broker's stdin.

- **Action Broker hardening:** the packaged broker now denies all IP address
  traffic and high-risk syscall groups in addition to its AF_UNIX-only and
  read-only-home sandbox.

- **Signed organization packs:** packs can be signed with OpenSSH keys
  (`wayexpand pack sign`, `pack verify`); the signature covers a digest of
  the manifest and every snippet file. Manifests can declare
  `capabilities`, `allowed_actions`, and `min_wayexpand_version`, and packs
  using undeclared capabilities are rejected. In safe mode,
  `require_signed_packs` with a root-owned `pack_signers_file` gates
  `pack import` and manifest packs in the fleet pack layer, which load as
  managed snippets separate from the user's library.
- **Library sync:** `wayexpand sync init` / `wayexpand sync` keep the
  snippet library in Git with any remote, validating before commit and
  after merge and never committing private files; GUI Library → Sync.
- **Interactive form snippets:** `{{field:name}}`, `{{field:name=default}}`,
  and `{{choice:A|B}}` open a form when the snippet fires; the filled text is
  typed only if focus returns to the exact original toplevel. Backends without
  a bounded window identity refuse to open forms rather than risk inserting
  into a different window; `wayexpand explain` reports when the capability is
  unavailable.
- **Local usage statistics:** `wayexpand stats` reports expansions,
  keystrokes avoided, most-used and unused snippets, and risky triggers from
  a local, ID-only `usage-stats.json` (`settings.usage_stats = false` to
  disable).
- **Template variables:** `{{snippet:TRIGGER}}` includes, allowlisted
  `{{env:NAME}}`, and opt-in `{{clipboard}}`, each policy-controllable; see
  docs/TEMPLATES.md.
- **`wayexpand explain`:** explains, check by check, why typed text would or
  would not expand, using the daemon's live state.
- **Trigger aliases:** `aliases = [...]` gives one snippet several triggers;
  Espanso `triggers:` lists now import.
- **Matching:** canonically equivalent (NFC/NFD) triggers match.
- **Testing:** IBus failure-mode tests over real D-Bus dispatch, cargo-fuzz
  targets for every untrusted-input parser (CI and nightly), a reload
  concurrency test, and a daemon soak test.
- **Daemon status after reconnects:** the control socket no longer keeps
  serving a capability-less "connected" status after an output reconnect;
  `doctor` and the GUI previously reported every capability as false and
  `backend_mode=unknown` until something else changed.
- **Event-driven daemon wakeups:** finished command expansions,
  control-socket requests (pause, resume, reload, stop, insert), and
  shutdown signals wake the evdev and input-method reactor immediately
  through an eventfd. The 10 ms polling while commands ran is gone; the
  250 ms idle tick remains for reload debounce, output retries, and focus
  snapshots.
- **Maintainability:** the GUI, CLI, and daemon entry points were split into
  feature modules (GUI `main.rs` 6,168 → 549 lines, CLI 3,411 → 304, daemon
  2,805 → 516), the daemon reactor loop is now a sequence of named steps, and
  core configuration errors, organization policy, and secure storage live in
  their own modules. A `config_edit` benchmark records the cost of a GUI edit
  at library scale.
- **IBus safety and correctness:** Action Broker results now reach IBus
  clients. Command workers wake a completion thread that emits each result
  and records it as applied only after emission; previously queued results
  were never delivered. Capture fails closed until a field reports a known
  non-sensitive content type, and unknown purposes plus the `PRIVATE` and
  `HIDDEN_TEXT` hints are treated as sensitive. Replacements are refused
  unless the client's surrounding text confirms the trigger before a
  collapsed cursor. Immediate triggers no longer delete one extra
  character. The IBus route reports `atomic_replace = false`, is governed
  as `ibus` rather than `input-method-v2` in `allowed_backends`, and is
  checked against `require_atomic_replace`/`require_sensitive_focus`.
  Legacy X11 keysyms (Greek, Cyrillic, Arabic, Hebrew, keypad) now match.
  **Policy migration:** sites with a non-empty `allowed_backends` that rely
  on IBus must add `"ibus"`.
- **IBus service in packaged builds:** packages build the IBus service in
  the same Cargo invocation as the daemon, which enables zbus's tokio
  backend. In that mode `CreateEngine` made a blocking zbus call from inside
  its handler and panicked, so no engine could be created. D-Bus handlers
  now use the async API, and engine calls are handled serially in arrival
  order. A peer-to-peer D-Bus test now exercises the real factory, key
  events, and broker completion delivery.
- **Correctness fixes:** `app_id_glob:` filters once again let `*` match an
  empty string (`*foo` matches `foo`); broker-routed commands no longer fail
  when an action takes longer than 100 ms, and a broker hang-up mid-response
  is reported immediately instead of spinning until the deadline; output
  preflight counts a reinserted delimiter as one character and no longer
  rejects zero-length cursor moves; the serialized output worker's
  completion wait is bounded so a wedged backend triggers reconnect instead
  of stalling the daemon; a command flooding stderr can no longer delay its
  own timeout.
- **Daemon lifecycle:** hardened control-socket startup now remains compatible
  with systemd user sandboxes where `openat2` is unavailable, and libei portal
  shutdown constructs its Tokio timer inside the runtime instead of panicking
  during daemon exit.
- **Certification claim integrity:** route contracts now distinguish sensitive-
  field handling implemented by WayExpand, protocol signals, compositor
  observations, and reviewed certification. Generated support/certification
  tables expose those states separately, and GNOME input-method-v2 wording no
  longer incorrectly claims that detection is absent.
- **Production-readiness hardening:** Action Broker now drains and joins all
  in-flight connection tasks before shutdown so accepted audit events are
  flushed; systemd namespace ownership checks, audit sandbox coverage, and
  broker end-to-end shutdown tests were strengthened.
- **Certification integrity:** compositor certification tooling now validates
  client coverage by token boundary rather than loose substring matching, so
  an identifier such as `notgtk` cannot satisfy the required GTK coverage.
  Missing `jq` is reported before the certification contract is evaluated.
- **Documentation accuracy:** input-method-v2 and IBus/Fcitx documentation
  now consistently identify compositor certification and active IME/preedit
  support as incomplete. Documentation contracts prevent unsupported
  "fully tested" or universal-composition claims from returning.
- **Documentation accuracy:** first-run guidance now points users to a normal
  text field and Desktop details instead of implying universal application
  coverage; the capture trade-off guide no longer presents uncertified
  input-method paths as covering a fixed percentage of users or "just working."
- **CI reliability:** release smoke tests, GUI/UI help checks, and RPM
  packaging are bounded by explicit timeouts; the systemd lane uses resilient
  Cargo network settings for hosted runners.
- **Documentation sprint:** refreshed the threat model, operations guide,
  sysadmin audit guidance, backend caveats, documentation index, and
  sensitive-field wording to match the integrated Action Broker, XDG state
  directory handling, current audit-health behavior, and uncertified
  compositor/IME boundaries. Added contracts to prevent the old
  pre-integration claims from returning.
- **Output correctness:** serialized desktop injection now waits for a
  per-operation backend acknowledgement before the engine commits expansion
  and undo state. Late libei/wlroots failures therefore reach the transaction
  caller instead of being mistaken for successful queue admission; regression
  tests cover delayed completion and backend failure propagation.
- **Runtime safety:** configuration reloads now preserve active composition
  state, preventing matching from resuming during dead-key, Compose, or IME
  preedit. Action Broker IPC now observes the configured command deadline and
  shutdown cancellation instead of relying on longer fixed socket timeouts.
- **Output preflight:** injector capabilities are now enforced before any
  erase/insert operation. Unsupported replacement lengths, Unicode, and cursor
  placement fail closed before the target application is modified.
- **Efficiency:** application-ID glob matching no longer allocates a character
  vector per candidate, and bounded command stderr continues draining until
  the pipe is empty after the retention cap is reached.
- **GUI wording:** first-run daemon status now says “Not enabled” instead of
  presenting the expected pre-setup state as a failure. Case propagation is
  labeled by its actual behavior, and app-filter syntax is kept behind an
  advanced disclosure.

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

## [1.3.3] - 2026-10-03

Release v1.3.3. Move the unreleased entries above into this section before publishing.

## [1.3.2] - 2026-10-03

Release v1.3.2. Move the unreleased entries above into this section before publishing.

## [1.3.1] - 2026-10-03

Release v1.3.1. Move the unreleased entries above into this section before publishing.

- Restrict the packaged Action Broker and its child actions to Unix-domain
  sockets and document the required no-network service sandbox.

## [1.3.0] - 2026-10-03

Release v1.3.0. Move the unreleased entries above into this section before publishing.

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

[Unreleased]: https://github.com/cyberducttape/wayexpand/compare/v1.3.3...HEAD
[1.3.3]: https://github.com/cyberducttape/wayexpand/compare/v1.3.2...v1.3.3
[1.3.2]: https://github.com/cyberducttape/wayexpand/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/cyberducttape/wayexpand/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/cyberducttape/wayexpand/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/cyberducttape/wayexpand/compare/v1.1.2...v1.2.0
[1.1.2]: https://github.com/cyberducttape/wayexpand/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/cyberducttape/wayexpand/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/cyberducttape/wayexpand/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v1.0.0
[0.2.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v0.2.0
[0.1.0]: https://github.com/cyberducttape/wayexpand/releases/tag/v0.1.0
