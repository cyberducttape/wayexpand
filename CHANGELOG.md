# Changelog

All notable changes to WayExpand are documented here.

## [Unreleased]

- Matcher preview rebuilds now seed matcher context from text already shown in
  the preview while discarding historical match results. Editing or changing
  preview context can no longer apply a trigger embedded in an earlier
  expansion for a second time.
- Escaped control characters in the daemon's reported configuration path and
  made duplicate status fields unavailable to consumers. Custom filenames or
  malformed status can no longer spoof pause, health, or capability values in
  the line-oriented diagnostics protocol.
- Launchpad synchronization now retries its SSH authorization probe on
  transient failures before giving up, and retries remote branch and tag
  lookups so a temporary path-translation failure cannot bypass push retries.
- Shell validation now checks the workflow retry assertion without triggering
  ShellCheck's warning about intentional literal shell syntax.
- Backup creation now compares the copied bytes against the opened source, so
  same-size concurrent edits are detected even when filesystem timestamps
  have coarse resolution. This also removes a Launchpad test failure caused by
  both writes landing within one timestamp tick.
- Preserved a fresh isolated KWin report for commit `6e8d3b6ae321`: output,
  focus safety, reload, restart, and command completion passed; keysym-fallback
  latency measured 12,463 µs p50 and 12,562 µs p95/p99. Unicode, physical
  keyboard capture, and suspend/resume were not tested, so the report is
  partial compatibility evidence, not production certification.
- Configuration watcher events now reload only when they concern the
  configuration file, its `snippets.d` layers, or the fleet directories.
  Since watcher events started forcing a reload, any write in the
  configuration's directory (usage statistics, preferences, logs) rebuilt the
  engine and discarded running commands, so command-backed expansions could
  never complete; the headless KWin `command-expansion-completes` scenario
  caught it.
- Waiting for a command that closed its stdout or stderr while still running
  no longer busy-polls the hung-up pipe; the worker previously used a full CPU
  core until the command exited or timed out. A command-backed match dropped
  because the expansion queue was full is now reported in command failure
  diagnostics like the deferred path.
- Configuration watcher events from read-only access (opening or reading the
  file) no longer request a reload. Since watcher events started forcing a
  reload, the reload worker's own read of the configuration re-triggered the
  watcher, rebuilding the engine every debounce interval and discarding
  partially typed triggers.
- The action broker now accepts root-owned or user-owned sticky directories
  such as `/tmp` as protected ancestors of private configuration and audit
  paths. Its SIGTERM integration test places private fixtures under `/tmp`, so
  Debian package builds do not depend on ownership of `TMPDIR` or the builder's
  `$HOME`.
- Preserved a current-source, headless KWin smoke report for the broker-test
  fixture fix. The isolated insertion, focus, reload, restart, and refusal
  checks passed; keyboard capture, Unicode on the keysym fallback, and
  suspend/resume remain untested, so this is not compositor certification.
- Preserved a fresh `doctor --certify --json` report for commit
  `1a274fbd6fa7`. The isolated KWin clients passed 12 output/focus scenarios;
  20 small keysym-fallback insertions measured 12,375 µs p50 and 12,531 µs
  p95/p99 inside the injector. Keyboard capture, Unicode insertion, and
  suspend/resume remain untested. The report is partial compatibility evidence,
  not production certification.
- Made the nested fleet-file signature regression test replace files using the
  same atomic-rename pattern as configuration saves. It no longer depends on
  sub-second in-place timestamp updates from the build filesystem.
- Debounced configuration watcher events now start the background reload even
  when coarse filesystem timestamps leave the metadata signature unchanged.
  This preserves detection of same-size in-place edits without reading config
  contents on the keyboard reactor.
- Verified the KWin window tracker in a live Plasma Wayland session on KWin
  6.6.6: it received the focused window identity, rejected an untrusted D-Bus
  focus event, and detected removal of its temporary script. This component
  check does not certify keyboard capture or text insertion.
- The Diagnostics dashboard now gives explicit active-route assessments for
  capture availability, insertion safety, compositor/device permissions, and
  focus visibility. It also shows recent content-free expansion outcomes,
  including policy, command, and evdev safety-gate failures, acknowledged
  insertions, and the stage where blocked or uncertain output attempts stopped.
  The trace never records typed or inserted text, and distinguishes backend
  acknowledgement from proof of client display. It separates administrator
  policy blocks from command results discarded after sensitive focus or pause.
  Insertion readiness also stays unknown when the daemon omits its full-Unicode
  capability instead of treating missing evidence as support. The dashboard
  warns when the running daemon is stale or its build identity cannot be verified,
  and keeps its sensitive-field certification note translated in every GUI language.
- Removed KWin and Mutter from the input-method-v2 certification target list
  after live KWin probing and protocol-support checks showed those compositors
  do not advertise the required manager. No declared desktop currently has a
  production-eligible route, so the release certification workflow now fails
  explicitly before runner discovery instead of treating the impossible KWin
  target as a valid gate. Compatibility and headless evidence remain unable to
  satisfy production certification. Corrected the GNOME setup guide and
  certification regression fixture so neither advertises input-method-v2 on
  Mutter.
- Certification evidence schema 3 now labels compatibility and production
  scope separately. Release checks and support-table generation require a
  production-scoped report, a production-eligible route, and an explicitly
  required matrix target; a complete compatibility run cannot promote a
  desktop.
- Added current-source KWin 6.6.6 headless certification evidence. The
  supported output/focus scenarios passed, but keyboard capture, Unicode on
  the negotiated keysym route, suspend/resume, and production desktop
  certification remain unverified.
- The pinned matcher playground now tests the current unsaved draft against the
  rest of the library, including aliases, match rules, app filters, and reusable
  snippet includes. Command-backed snippets remain disabled there. Successful
  tests explain the trigger that fired; invalid previews show safe template
  validation errors instead of looking like replacement text.
- The template palette can insert references to reusable static snippets.
  Template validation summaries no longer echo user-provided variable names,
  formats, or random-choice values into GUI diagnostics.
- ConfigStore reload now retains full validation on the candidate snapshot and
  verifies its exact source revision with a second secure file read, without
  parsing and validating the same TOML a second time. A benchmark and regression
  test cover the revision check; exploratory 10,000-snippet measurements show
  a substantial reduction in reload validation time.
- Added a Criterion configuration reload group that measures the ConfigStore's
  stable two-read validation separately from file-load/validation plus matcher
  construction. Added a seeded concurrent snapshot-reader stress test, varied
  its seeds in the existing parallel CI lane, and added a dedicated CI suite
  for concurrency and injected storage, worker, output, and audit failures.
- Replacement assurance is now explicit in backend route recommendations,
  generated support tables, daemon status schema 13, `doctor`, and GUI/TUI
  status. Live status distinguishes verified target-text checks,
  focus-tracked best-effort replacement, unverified replacement, blocked
  replacement, and unavailable output. Strict mode refuses keystroke
  replacement before typing; regression coverage now includes hard-linked
  configuration lock files. Security documentation maps the boundary tests
  and states their real-desktop certification limits.
- Daemon soak reports and isolated KWin certification now share source-tree
  cleanliness checks scoped to build and harness inputs. Unrelated generated
  files no longer make their evidence appear to come from a dirty source tree.
- Added a script regression check proving unrelated reports are ignored while
  untracked source inputs still mark the checkout dirty.
- Fixed a POSIX shell compatibility issue in the isolated E2E latency scenario
  so the repository-wide ShellCheck CI step can validate it.
- Certification contract failures now print the captured machine-readable
  report, preserving the reason and scenario outcomes when the private KWin
  harness exits intermittently.
- Refreshed the preserved October 25 KWin smoke report with current-source
  evidence for all insertion/focus scenarios and backend-only latency samples;
  the report remains partial and explicitly records untested paths.
- Extended the GUI benchmark to measure one draft keystroke plus live-preview
  computation at 100, 1,000, 5,000, and 10,000 snippets; recorded methodology
  and host-specific measurements in `docs/BENCHMARKS.md`.
- The matcher now tracks non-ASCII scalars in its bounded input buffer and
  skips repeated NFC scans for all-ASCII input. Regression coverage exercises
  append, eviction, backspace, and reset; before/after benchmarks are recorded.
- Configuration stores now publish the validated configuration and generation
  as one coherent snapshot, with replacement notifications ordered alongside
  updates. The IBus service reads a single snapshot when applying reloads;
  separate configuration and generation accessors are deprecated to prevent
  callers from accidentally combining different versions. Full subscriber
  channels are documented and tested as coalesced invalidation signals.
- Strict-path Action Broker commands are opened and validated once, then
  launched through the pinned executable descriptor. This closes the
  validation-to-spawn path replacement race, including for shebang scripts.
- Daemon status now distinguishes usage events collected, persisted, and
  known dropped across the bounded engine buffer, writer queue, and final
  persistence retries, and reports writer-worker failures. Usage statistics
  are documented as best-effort aggregates rather than an exact audit ledger.
- IBus now retires an engine instance after an output signal fails, preventing
  further expansion while a delete/commit transaction may have left the
  client document partially changed. Only a new `FocusIn` clears retirement;
  an `Enable` callback alone cannot resume matching.
- Isolated KWin certification now determines source cleanliness from build and
  harness inputs, so unrelated untracked files do not invalidate otherwise
  matching CLI/daemon identity evidence.
- Exact-tag release version stamping now rejects staged changes and untracked
  files as well as unstaged edits, preventing modified source trees from being
  labeled as clean releases. Builds using an externally supplied commit
  identity also remain development builds, even on a tagged checkout.
- Git remote status and diagnostic output now redact the entire query string,
  avoiding credential leaks through vendor-specific parameter names or
  alternate query separators.
- `wayexpand doctor --certify` now reports p50/p95/p99 injector latency from
  20 verified small-text replacements in the isolated KWin harness. The sample
  excludes physical capture and client paint latency and is informational; it
  does not change backend pacing or claim an SLO pass.
- Isolated desktop certification now rejects duplicate or invalid scenario
  records, preventing malformed harness output from overwriting a failure with
  a later pass.
- Source-checkout certification reports whether the tested tree was clean;
  dirty-tree results no longer count as an identified build, and duplicate or
  missing build metadata cannot overwrite or satisfy the identity check.
- Added backend failure-injection coverage proving libei and wlroots key
  sequences stop immediately after a mid-replacement send failure.
- GUI command previews now refresh systemd user-manager availability instead
  of caching the startup result, so a later-started manager cannot leave
  preview execution on the stale unsandboxed path. Once detected, a transient
  socket failure keeps previews on the sandbox route and fails closed.
- Expansion failure traces now label the gate as output admission, avoiding
  the implication that a successful admission always verified focused-client
  identity; route-level focus assurance remains reported separately.
- Privacy-filtered `wayexpand support` summaries now retain the compositor
  protocol-probe statuses, so support diagnostics preserve why an integration
  route is unavailable without including raw daemon or configuration details.
- `wayexpand doctor --json` now includes bounded live protocol-probe results
  for input-method-v2, wlroots virtual keyboard, and the KWin window tracker.
  This separates compositor availability from static backend implementation
  status without requesting portal consent or implying certification.
- Added an opt-in KWin input-method E2E path with a text-input-v3 target and
  simulated key capture for replacement and password-field suppression. The
  current headless KWin setup does not expose input-method-v2, so these checks
  remain conditional and are not counted as certification or CI coverage.
- Certification harness reports now include bounded daemon readiness details
  on unverified startup cases, so archived results explain why a scenario
  could not run without requiring a separate log lookup.
- Daemon status schema 11 publishes a canonical `route_id`; the GUI now
  resolves route diagnostics from that stable ID instead of recombining source
  and injection fields.
- Preserved a current-head private KWin certification run: 12 output, focus,
  reload, restart, and refusal checks passed. The report remains partial because
  keysym fallback, physical keyboard capture, and suspend/resume leave required
  behavior untested; see `docs/evidence/doctor-certify-6551bb4b7.json`.
- Engine construction now reuses parsed app filters, hotkey chords, and undo
  chord from the same validation pass that builds effective triggers, removing
  separate runtime parsing of those validated fields.
- Configuration saves now distinguish a failed parent-directory sync after
  atomic replacement from a pre-commit failure, warning that the new file is
  visible but crash durability is uncertain and should be verified before retry.
- Configuration backups now report when a parent-directory sync fails after
  the complete backup has been created, so an error does not imply the backup
  file is absent.
- Portal-token persistence now identifies a directory-sync failure after token
  replacement as an uncertain durability outcome, rather than implying the
  restoration token was not installed.
- Snippet-pack signing now reports when the signature was replaced but the
  parent directory could not be synced, so callers can distinguish that
  uncertain durability outcome from a failed signature write.
- GUI preference saves use collision-resistant temporary names and sync their
  parent directory; post-replacement sync failures identify that preferences
  were saved with durability still uncertain.
- Configuration backups compare the opened source file's identity and change
  timestamps before and after copying, rejecting a same-size concurrent edit
  instead of preserving a potentially inconsistent backup.
- Snippet-pack signing no longer joins its stdin writer after the signing
  process times out, and bounds the writer completion handshake after exit.
- Preserved a current-build `doctor --certify` report for commit
  `bdf55e97993c`: 12 isolated KWin 6.6.6 output, focus, reload, restart, and
  refusal checks passed using libei keysym fallback. Unicode insertion,
  physical keyboard capture, and suspend/resume remain untested; the report is
  partial and does not certify the active desktop.
- Preserved the current-head `doctor --certify` report for commit
  `3e3ea129d7a5`: 12 isolated KWin 6.6.6 output, focus, reload, restart, and
  refusal checks passed using libei keysym fallback. Unicode insertion,
  physical keyboard capture, and suspend/resume remain untested; the report is
  partial and does not certify the active desktop.
- A timed-out daemon usage-statistics clear now says its outcome is unknown and
  directs the user to inspect statistics before retrying; the queued clear can
  still finish after the caller's response deadline.
- Mandatory Action Broker audit now applies bounded backpressure while the
  queue is full and uses one deadline for queue admission and durable-write
  confirmation; queue exhaustion remains visible as a counted, explicit audit
  failure.
- Mandatory audit now persists an action-start marker before executing a
  command and records the result afterward. If the start marker cannot be
  confirmed, the broker refuses to run the action.
- Certification report unit tests now use explicit build identities, so they
  remain meaningful for source archives without Git metadata; the KWin timeout
  harness now gives its fake CLI and daemon matching identities as required by
  the harness contract.
- Preserved a current-build `doctor --certify` report for commit
  `480206ce7d1f`: 12 required isolated KWin 6.6.6 output, focus, reload,
  restart, and refusal scenarios passed using libei keysym fallback. Unicode,
  physical keyboard capture, and suspend/resume remain untested; this evidence
  is partial and does not claim active-desktop certification.
- Process supervision now exposes child stdout without exposing raw `Child`
  lifecycle methods, preventing callers from reaping a leader before its group
  cleanup and invalidating the PID-reuse safety guarantee. Raw process-group
  killing by caller-supplied PID is no longer exported.
- Pack signing now refuses symlink/non-file signature targets and atomically
  replaces regular signatures with private, synced output.
- Added an Action Broker regression test proving continuous stderr output cannot
  starve the configured child-process deadline.
- The Action Broker now poisons its audit writer if a failed batch cannot be
  rolled back and synced, preventing later retries from reporting success over
  a log with uncertain contents.
- Mandatory Action Broker audit now syncs file creation and rotation directory
  entries before acknowledging persisted events.
- Preserved a current-build `doctor --certify` report for commit `611017a88cec`:
  11 isolated KWin 6.6.6 insertion, focus, reload, restart, and refusal checks
  passed using libei keysym fallback; Unicode insertion, physical keyboard
  capture, and suspend/resume remain untested. The report explicitly remains
  partial and does not claim active-desktop certification.
- `wayexpand new --gui` now bounds form-helper runtime and output, and kills
  the helper's process group on timeout or when a descendant keeps stdout
  open after the GUI exits.
- Preserved a build-identity-checked isolated KWin certification report from
  commit `552f1c1377eb`; all 12 executed insertion, focus, safety, and identity
  checks passed, with keyboard capture, Unicode insertion, and suspend/resume
  still untested.
- Isolated `doctor --certify` now records the commit of the CLI and daemon
  binaries it exercises, and rejects evidence produced by a different build
  than the reporting CLI.
- Process supervision now invalidates and cleans up the process group as part
  of `reap()`, preventing a later drop from signalling a recycled group ID
  when callers omit an explicit cleanup call; a regression test verifies a
  descendant is terminated when `reap()` is called directly.
- Preserved a current-source `doctor --certify` run from commit `2111dba66512`:
  11 isolated KWin insertion/focus checks passed using libei keysym fallback;
  Unicode insertion, physical keyboard capture, and suspend/resume remain
  untested. The artifact explicitly does not claim desktop certification.
- Audit rotation now replaces the previous backup atomically, preserving it if
  moving the active log fails.
- Clipboard worker startup and disconnected-job failures are now visible in
  diagnostics; a failed enqueue is counted and fails closed without waiting for
  the clipboard deadline.
- `doctor --certify` now validates its argument shape without removing user
  arguments during dispatch; CLI regression coverage confirms invalid
  positional arguments stop before the isolated compositor is started.
- Added input-method-v2 fault injection at the atomic replacement flush
  boundary, verifying the delete, commit, and `done` requests are queued before
  an acknowledgement error is returned. Its errors remain non-replayable.
- Preserved the current 12-scenario isolated KWin doctor-certification JSON
  report, including each verified result and its explicit headless-route limits.
- `wayexpand doctor --certify` now reports word-boundary, cursor, multiline,
  command completion, reload, and restart scenarios alongside its insertion
  and focus safety checks; missing scenario evidence keeps the report incomplete.
- The headless GUI screenshot runner now terminates its optional daemon and
  other private-session processes before removing temporary state.
- The daemon soak now enforces configurable peak RSS, descriptor, and thread
  growth limits after warmup, and records baseline, peak, and final counts.
- `wayexpand doctor --certify` now includes in-flight focus-change and target-
  close checks. The partial compatibility report requires these safety
  scenarios to pass before reporting a successful run.
- The GUI now flags a running daemon built from a different commit and explains
  that the user service must be restarted before relying on the editor's newer
  behavior; matching or unidentifiable build identities are kept distinct.
- Focus checkpoints now guard every libei text chunk and synthesized key, and
  every wlroots output key. The backends flush between chunks or key events so
  a focus change cannot send the remaining replacement into another window;
  regression tests cover interruption before the next output event. A daemon
  actor regression verifies partial output is not replayed and the worker
  remains usable after focus is refreshed. The checkpoint API and warning text
  now describe both bounded text chunks and synthetic keystrokes.
- `wayexpand doctor --certify` runs an isolated KWin test session and verifies
  received output from real Wayland test clients, reporting insertion, focus,
  and Unicode outcomes without claiming to certify the active desktop. The
  getting-started guide now explains how to run it and save its JSON report.
- Atomic input-method replacement errors after submission are now reported as
  an unknown application outcome. The daemon refuses automatic replay when a
  compositor may have applied the replacement but its acknowledgement was
  lost, and discards its deferred trigger reservation before the next input.
- Preserved a `doctor --certify --json` run from build `c9d9f3eac46c` on KWin
  6.6.6, including its partial outcomes and explicit keysym-fallback limits.
- Recorded an 11-scenario headless KWin run from `99b4bdc50`; output, focus,
  reload, and restart passed. Physical capture and Unicode remain outside the
  evidence. See `docs/evidence/kwin-headless-99b4bdc50.json`.
- The GUI diagnostics dashboard no longer reports permissions as available when
  environment probes say they are not applicable; granted, required, and
  unreported/not-applicable states are now distinct.
- GUI permission health now follows the validated live daemon route, including
  explicit permission-required and portal-revoked states.
- Backend diagnostics now report a grant only for permission-bearing routes;
  compositor protocol and window-tracker availability no longer imply an OS
  permission grant.
- Isolated desktop certification reports now identify the private session as
  headless KWin instead of KDE Plasma, so saved evidence cannot imply that the
  Plasma desktop shell was exercised.
- Release smoke testing now builds the binary archive, verifies its checksum,
  and checks that runtime binaries and certification helpers are present while
  excluded repository content stays out.
- Certification matrix drivers now reject unknown, duplicate, and unsafe
  keyboard-layout profile names before building per-cell log paths.
- The IBus availability probe now terminates command descendants before
  draining their inherited output pipe, keeping discovery bounded.
- GUI compositor-version probes now read output without blocking, cap retained
  data, enforce their deadline, and terminate descendants that inherit the
  probe pipe.
- Isolated `wayexpand doctor --certify` reports now include the exact
  WayExpand build version, commit, and run timestamp, making saved compatibility
  results traceable to the binary that produced them.
- Scoped KWin protocol-availability documentation to the tested headless
  session; a default-configured private compositor run is not a universal
  capability result for a user's configured virtual keyboard.
- GUI command-preview transient units now grant the same resolved XDG config
  and state paths as installed daemon units, with paths quoted against spaces
  and systemd specifier expansion.
- `doctor --json` now describes an unhealthy readiness or runtime result
  accurately instead of implying the configuration or control socket failed.
- In managed safe mode, snippet-pack imports can no longer replace the
  organization’s required signer list with a caller-supplied `--signers` file.
- Snippet-pack signature hashing now fails on directory enumeration errors and
  hashes non-UTF-8 filenames without lossy-name collisions.
- Compositor certification evidence now requires the exact declared keyboard
  layout profile set, rejecting duplicates and undeclared profiles.
- The GUI’s managed-action picker now bounds broker-policy reads, avoids
  blocking on special files, refreshes at most every two seconds, and shares
  cached action IDs without cloning the whole list on each frame.
- CLI help and the manual now list isolated `doctor --certify` separately and
  document pack operations and the reviewed-revision requirement for executable
  snippets during Git sync.
- The Debian/RPM packaging job frees Debian build outputs and its duplicate
  extracted source before running the RPM workspace tests, reducing peak
  temporary storage use.
- Fixed the wlroots virtual-keyboard output route typing the wrong keys. It
  sent XKB keycodes where the protocol expects evdev codes, so every client
  decoded each key 8 positions off: BackSpace never erased and replacements
  came out as wrong characters or nothing. The generated keymap now starts at
  XKB keycode 9 and keys go out as evdev codes.
- On desktops without KWin, the KWin window tracker's reprobes republished
  "no window" on every retry, and each republish cleared the partly typed
  trigger and discarded pending command results. A focus snapshot that names
  the same target (still no window, or the same exactly identified window
  with a new title) no longer resets the engine; a title-only change still
  updates `title_contains:` filters. Updates without an exact window instance
  still count as focus changes.
- Added a headless Sway end-to-end harness (`scripts/test-sway-e2e.sh`) and a
  CI job for it. It drives the production daemon and wlroots output into real
  Wayland windows in a private Sway, with capture simulated through stdin and
  focus untracked. The KWin and Sway harnesses now share their scenarios in
  `scripts/e2e-lib.sh`, which the certification packages install beside the
  KWin scripts.

- An installed `wayexpand doctor --certify` now tests the certification
  helpers installed beside it. Before, a binary built from a source checkout
  (such as a user install) preferred that checkout's script if it still
  existed, which built and tested a debug build of whatever commit the
  checkout was then at. A harness whose private KWin session times out is now
  reported as a timeout rather than as missing desktop dependencies.
- Force-stopping a form helper's transient unit no longer passes
  `--kill-whom=`, which systemd before 252 rejects; every process of the unit
  is already `systemctl kill`'s default target.
- The clipboard prefetch counted a read that expired before it started as two
  deadline misses.
- A daemon status test now fills every latency profile and the whole failure
  history, so the variable-length status fields are proven to fit the
  client's 8 KiB read limit.

- Git synchronization now writes large index updates through the supervised
  nonblocking stdin pipe, so a stalled Git process or inherited pipe cannot
  strand a writer-thread join beyond the command deadline.
- Added regression coverage for bounded input/output queues: overflow resets
  stale keyboard input, fresh input resumes afterward, and the output actor
  accepts new transactions once queued work drains. The production acceptance
  checklist now links to these exact scenarios.
- GUI latency diagnostics now label matcher and backend injection latency
  explicitly, and accessibility tests cover the keysym-fallback warning and
  the separate percentile rows.
- GUI command-preview warnings now distinguish direct programs run in a
  transient systemd unit with WayExpand's service sandbox from programs run
  locally when no user manager is available.
- Human `wayexpand doctor` output now includes latency percentiles by negotiated
  output mode and replacement-size bucket, making keysym-fallback costs for
  longer expansions visible without changing the configured pacing.
- Cancelling a form helper launched through `systemd-run` now force-stops all
  processes in its unique transient unit after stopping the wrapper, preventing
  late unit starts and GUI descendants in the separate systemd cgroup from
  surviving daemon cancellation or timeout.
- Configuration integrity polling now fingerprints file metadata rather than
  reading the primary file and every fleet layer on the keyboard reactor.
  Full contents are still read and validated by the background loader before
  a reload is promoted, avoiding synchronous disk reads during live input.
  Fleet signatures are captured before a build so a layer edited before the
  reactor applies that build is detected and reloaded instead of being hidden
  by a fresh post-build signature.
- The Action Broker shutdown integration test now keeps its Unix socket in a
  short private runtime path, so Debian package builds with deeply nested
  staging homes do not exceed the platform's socket-path limit.
- The headless KWin E2E runner now rejects a timed-out compositor session even
  when it already wrote passing scenario results, and includes the compositor
  log tail in the failure report. CI covers this timeout-after-results case.
- Human `wayexpand doctor` output now shows the active capture/output route,
  negotiated injection mode, keysym-fallback warning, sensitive-field signal,
  and separate backend-injection versus matcher latency percentiles. It also
  flags a stale or protocol-incompatible daemon with its restart guidance.
- The isolated `wayexpand doctor --certify` report now records the test
  daemon's negotiated insertion mode, backend detail, capture source, and
  sensitive-field signal alongside the observed scenario results.
- `wayexpand doctor --certify` now attempts real UTF-8 insertion in its isolated
  KWin test client when the negotiated libei route offers direct text input.
  The report marks Unicode as passed only after the client receives the exact
  accented and Japanese output; keysym-only routes show it as not tested.
  Human reports now identify the desktop without JSON quoting and use concise
  scenario labels. CI exercises the report contract separately from its
  mandatory E2E scenarios.
- A successful action whose required audit sync fails now returns an explicit
  “completed; verify before retrying” error instead of a generic broker error.
  Existing action failures remain visible, with the separate audit failure in
  broker health and the service journal. A required-audit acknowledgment
  timeout now marks audit health unhealthy as well.
- Headless KWin CI now preserves failure diagnostics and reports the EIS relay
  startup error, so compositor-specific integration failures can be diagnosed
  from the workflow artifact.
- Action Broker shutdown now cancels and reaps in-flight process groups, then
  audits the cancellation, instead of waiting for each action's full timeout.
- Action Broker shutdown bounds its audit-writer join and reports when a
  stalled filesystem sync prevents queued audit events from being flushed.
- Updated the locked Windows clipboard dependency from yanked `clipboard-win`
  5.4.1 to 5.4.3. The workspace suite passes with the new lockfile and
  `cargo audit` no longer reports a yanked package; it reports only the
  already-documented informational zbus 5 unmaintained notices.
- CI now treats yanked locked dependencies as a failing security-audit result,
  preventing a later lockfile update from silently reintroducing one.
- CI now runs the production-daemon KWin/libei output and focused-window E2E
  scenarios in a private headless compositor session on every change. Capture
  remains simulated, so this catches output/focus regressions without claiming
  real-session certification.

- Preserved the 11/11 production-daemon KWin 6.6.6 headless end-to-end run as
  a redacted machine-readable artifact. It records the source commit and
  explicitly marks simulated capture and headless-session limitations; it is
  supporting evidence, not compositor certification.

- Increased the daemon control response limit to 8 KiB so the expanded
  diagnostics status, including privacy-safe failure traces, fits without
  weakening the bounded IPC read. Updated the CLI and daemon status contract
  fixtures to include the new fields and use the shared schema version.

- Added `wayexpand doctor --certify`: it runs the isolated headless KWin
  end-to-end harness and reports observed insertion and focus checks, while
  marking keyboard capture, Unicode insertion, suspend/resume, and production
  certification as unverified. Release, user, Debian, RPM, and Arch packages
  now include its small test helpers. The harness runs correctly from outside
  the checkout with a relative `CARGO_TARGET_DIR`, and restricts permissions
  only on its temporary work directory. It uses a private session and does
  not alter the active desktop. See `docs/CERTIFICATION.md`.
- Certification reports now distinguish a timed-out/incomplete harness from a
  scenario failure, and reject a successful partial result if the harness
  exits unsuccessfully during teardown or evidence collection.
- Daemon soak cleanup now terminates every process carrying its private runtime
  environment, including an orphaned daemon after a control-check failure. Soak
  evidence no longer records the host's potentially sensitive hostname.
- Daemon soak resource samples now verify and record the actual daemon PID and
  executable; this prevents a background shell wrapper from being mistaken for
  the process whose memory, threads, descriptors, and CPU are measured.
- Compositor-certification runner preflight failures now write the missing
  token permission or runner labels to the GitHub Actions job summary; the
  workflow passes actionlint's embedded shell analysis.

- Diagnostics now show the active capture and injection routes, detected
  compositor/version, route permission health, and rolling average injection
  latency. They also report sensitive-field capability alongside the explicit
  compositor-certification state. The GUI includes a bounded, session-local
  expansion failure trace with lifecycle stages and no trigger, inserted
  text, clipboard data, or raw keystrokes. The daemon status contract is now
  schema 10.
- The snippet editor's matcher explanation and trigger suggestions now sit
  alongside a form test playground with editable field values, rendered
  output, and cursor-position preview. Form values remain in the editor
  session. New optional packs provide parameterized Linux operations,
  Kubernetes, container, developer, and support-reply snippets; see
  `docs/PACKS.md`.

- Clipboard template reads now share one bounded worker and a single 150 ms
  deadline; a timed-out prefetch returns unavailable instead of starting a
  concurrent fallback read. Worker, in-flight, failure, deadline, and duration
  metrics are included in daemon status (schema 9).
- Usage statistics now treat only a missing file as an empty history. Malformed,
  oversized, symlinked, or unreadable files are reported and preserved, and
  background merges refuse to replace them until explicitly cleared.
- GUI duplicate-trigger feedback now uses an indexed trigger/alias lookup
  rather than scanning the full library on each frame; full configuration
  validation remains mandatory before save. Added scale benchmarks for live
  feedback and the asynchronous save pipeline from 100 to 10,000 snippets.
- Headless KWin end-to-end harness: `scripts/test-kwin-e2e.sh` runs eleven
  scenarios against the production daemon, libei output, and KWin window
  tracker in a private `kwin_wayland --virtual` session, with text checked
  in real Wayland windows. They cover expansion, word boundaries, cursor
  placement, multi-line text, an untypeable character being refused,
  `app_filter` by focus, focus changes and window closes during an
  expansion, config reload, and daemon restart. The helpers live in the new,
  never-packaged `wayexpand-e2e` crate. Capture is simulated, so results are
  not compositor certification; see `docs/INTEGRATION_TESTING.md`.
- Fixed: with `XDG_CONFIG_HOME` set, the daemon (in its default fleet mode)
  and `validate --fleet` loaded user snippet layers from
  `~/.config/wayexpand/snippets.d`, while the configuration file and
  `wayexpand sync` used `$XDG_CONFIG_HOME/wayexpand/`. Layers kept in sync
  were never loaded, and layers under `~/.config` were loaded but not
  synchronized. The user layer is now the `snippets.d` next to the default
  configuration.
- Fixed: `wayexpand-daemon CONFIG` failed with "multiple configuration paths
  supplied" when `WAYEXPAND_CONFIG` was set. A path argument now overrides the
  variable, as it does for the CLI.
- A hotkey of only modifiers (for example `Ctrl+Shift`) now reports that it
  needs a key, instead of "key chord is empty".
- Fixed: `{{local_date}}`, `{{local_time}}`, `{{local_datetime}}` and
  local date formats kept the time zone the daemon started with. glibc's
  `localtime_r` reads the zone only once per process, so after the system
  zone changed (travel, `timedatectl set-timezone`) expansions used the old
  offset until a restart. The zone is now re-read when it changes.
- Fixed: `{{hostname}}` was read once from `/etc/hostname`. It stayed stale
  after a rename until the daemon restarted, and was empty when the name was
  set by DHCP or transiently. It now uses the kernel's current host name.
- Sync status and diagnostics now also redact signature query parameters
  (`sig=`, `X-Amz-Signature=`) in remote URLs.
- Fixed: editing a large replacement in `wayexpand-ui` froze it. Keeping
  the caret visible re-measured the text for every character it skipped,
  which is quadratic in the length: 50,000 characters took 5.7 seconds per
  keystroke in a debug build. It now measures only what fits on the line.
- `wayexpand stats` says "1 day" and "1 expansion" instead of "1 days" and
  "1 expansions".
- Fixed (review of the latest changes):
  - `wayexpand upgrade-status`, certification, and `doctor --json` treated
    two builds without Git metadata as the same build, since both report
    their commit as `unknown`. After a distribution-package upgrade, a stale
    daemon therefore showed as current. An unknown commit now never counts
    as a match, and `upgrade-status` reports the new `unverified` status.
    Its text output no longer prints the status with JSON quotes.
  - The Espanso importer failed the whole import when an option it ignores
    contained integer map keys or `.nan`/`.inf`, both of which the previous
    YAML parser accepted. Ignored option values are now skipped unparsed.
    Alias-expansion bombs are still rejected, and a test now covers them.
  - `fuzz/Cargo.lock` still pinned `serde_yaml` and `unsafe-libyaml`.
- Fixed: `wayexpand sync` could commit and push in the wrong repository.
  `git -C` does not override repository variables such as `GIT_DIR`,
  `GIT_WORK_TREE` and `GIT_INDEX_FILE`, and git exports them while it runs
  hooks. A sync started from a hook, or from a shell with `GIT_DIR` set,
  therefore acted on that other repository. Sync now removes git's
  repository-local variables before running git.
- The serialized output worker's acknowledgement deadline now counts every
  keystroke a transaction sends, including the cursor step back over a
  reinserted delimiter.
- A new test pins a focus-safety invariant: while a transaction types, the
  daemon applies no focus change, so a queued transaction cannot pass its
  focus check in a window that gained focus meanwhile.
- Replaced the deprecated `unsafe-libyaml` Espanso importer dependency with
  the safe Rust `serde-saphyr` parser, preserving the bounded import path.
- Daemon and Action Broker processes now establish a private umask before
  creating threads; socket creation no longer changes a process-wide umask
  while workers are running.
- GUI undo restoration now uses stable-ID maps instead of repeated vector
  searches, keeping large-library undo operations linear.
- GUI import merging now indexes existing IDs and effective triggers, avoiding
  repeated full-library scans when importing large snippet sets.
- Upgrade notes:
  - `wayexpand-evdev.service` now starts only when the consent marker
    `~/.config/wayexpand/evdev-sensitive-fields-ack` exists, so installing
    or enabling the package cannot silently turn on raw keyboard capture.
    Existing evdev users must create it after reading
    `docs/BACKENDS_SENSITIVE_FIELDS.md`; until then the unit is skipped.
    `wayexpand doctor` reports an enabled but inert evdev unit, and
    `wayexpand setup --backend evdev` and `install-release.sh --enable
    --service=wayexpand-evdev.service` refuse with instructions instead of
    reporting a service that never runs.
  - `wayexpand sync` refuses a library that contains command snippets or
    hotkeys. Pass `--allow-executable`; content pulled from the remote
    additionally needs `--approved-revision` with the full reviewed commit.
- Fixed: a GUI save queued while another was in flight lost an edit to a
  snippet that was no longer in the saved configuration, even though the
  newer local edit is meant to win.
- Fixed: the release ancestry check compared each release tag with itself,
  so it could never fail. It now requires descent from the previous
  release tag, and the release job fetches full history for it.
- Writing a file with `wayexpand new` no longer replaces a file created
  concurrently, and the file is private (0600). The broker client rejects
  oversized frames before buffering them.

- Terminal UI (`wayexpand-ui`) polish:
  - Fixed: rows no longer drift right ("staircase") and multi-line
    previews no longer scramble the screen. The UI drew each line ending in
    a bare newline, which in raw mode moves down without returning to the
    first column. Each frame now paints every row in place at full width,
    with no screen clear between frames.
  - The list scrolls to keep the selection on screen, and long text is cut
    with `…` instead of wrapping.
  - The selected snippet's details (status, matching, tags, category,
    apps, command, file) and its full rendered preview sit beside the list
    on wide terminals and below it on narrow ones.
  - A header shows the file and whether the daemon is running or paused.
    Messages are colored by outcome. The bottom line lists the keys that
    fit, and `?` lists every key.
  - New keys: `PgUp`/`PgDn`, `Home`/`End` (`g`/`G`), and caret editing in
    prompts. `Esc` clears an active search before it quits.
  - `NO_COLOR` turns colors off.
- Documentation and presentation:
  - The README is shorter and leads with what WayExpand does. It states
    each compatibility caveat once, and adds a section on file templates
    (`wayexpand new`). The German README gains the same section.
  - The README screenshots show the current GUI.
  - The planning documents moved out of the repository root:
    `PROFESSIONAL_ROADMAP.md` is now `docs/ROADMAP.md`,
    `RELEASE_PROMOTION_GUIDE.md` is now `docs/RELEASE_PROMOTION.md`, and
    the Claude Code notes are now `.claude/CLAUDE.md`.
  - The bundled example `:sig` signs as "Your Name".
  - `scripts/make-social-preview.sh` builds a GitHub social preview image.
    `scripts/gui-screenshot.sh` gains a `preview` view and options for the
    color pack and a sandboxed daemon.
- Fixed: the Ubuntu package's test run failed on Launchpad builders. The
  sync tests need `git`, and the input-method Compose tests need an XKB
  Compose table, so both are now test-only build dependencies and the tests
  run under `LC_ALL=C.UTF-8`. The command output-limit test made its output
  with `python3`, which builders lack, so it timed out instead; it now uses
  `head` and `tr`.
- Fixed: the snippet list kept widening until it filled the window. The
  search row sized its field from a guessed button width; with the
  editor's button padding the row was wider than the panel, so the
  resizable panel grew to fit it on every frame (about 2 px a frame, and
  17 px a frame while a search term showed the clear button). The field
  now takes exactly the width left after the buttons.
- GUI polish:
  - Search moved from the crowded header, where it shrank to "Se…", to the
    top of the snippet list it filters.
  - The replacement editor now sits right below the trigger, with the
    template variable buttons under it, instead of below every other field.
  - Labels, triggers and typed text use each color pack's own main text
    color; egui's dim baseline had made labels fainter than the hints
    beneath them.
  - Open dialogs dim the window behind them.
  - The Category picker lines up with its text field.
  - The two first-run cards share one width.
  - The Matcher preview caption no longer runs off the edge (the full
    explanation is on hover).
  - "Production readiness" in diagnostics no longer draws its value over
    its label.
  - The form-snippet note in the preview is translated.
- `scripts/gui-screenshot.sh` renders the GUI in a private headless
  compositor for checking its look.
- The editor's Template variables section groups its buttons (Date and
  time, Unique and random, Fill in when expanding, Text and system) and
  adds the new formatted date and time, UUID, random, field, choice and
  literal-brace markers, each with a hover description of what it does and
  how to change it.
- New template variables. `{{date:FORMAT}}` formats any clock variable, with
  or without an offset, using strftime codes with English names (for example
  `{{local_date+1w:%a %b %-d}}`). `{{uuid}}` gives a new version-4 UUID and
  `{{uuid:NAME}}` repeats one value wherever that name appears in the same
  expansion. `{{random:hex:16}}` and the `digits`, `lower`, `alpha`,
  `alnum` and `password` kinds draw from the system's cryptographic random
  source; `{{random:A|B|C}}` picks one option. A malformed format or kind is
  a configuration error.
- "Use current app" in the snippet editor no longer always adds WayExpand
  itself. It took the first window a new KWin tracker reported, which is
  the editor where the button was clicked. It now waits up to 15 seconds
  for you to click the target app's window, skipping WayExpand's own
  windows and the desktop.
- The KWin window tracker no longer drops and reloads its script every
  time KWin is slow to answer. Under memory pressure (out-of-memory kills)
  KWin stalls for seconds, and each 1-second health-check timeout caused a
  disconnect, a script reload and lost focus state, which then timed out
  again. Unanswered checks are now tolerated for 10 seconds; a definite
  "script not loaded" answer still reconnects at once.
- Command snippets and hotkey actions no longer stay disabled after one
  failed worker restart. A reload started the new engine's command threads
  while the old ones still ran, which could exceed the unit's
  `TasksMax=32`. When that happened, every later reload kept commands off
  until the daemon restarted. The old workers now stop first, a failed
  start is retried every 30 seconds and on the next reload, and the
  enforced command policy is applied separately from that state.
- `wayexpand explain` now checks whether the output route can type the
  replacement. It had said "would expand" for a snippet that libei's
  keysym fallback refuses, such as one containing a pasted no-break space,
  which can only be typed as ASCII. The explanation names an invisible or
  look-alike character (no-break space, zero-width space, curly quote) and
  its position; other characters are not revealed. Status gains
  `output_unsupported_total` and `output_last_unsupported` (`unicode`,
  `too-long`, `cursor`), because these refusals were not counted anywhere.
- File templates: `wayexpand new` fills in a snippet's fields and writes the
  result to a file, so Dockerfiles, Ansible playbooks, Compose files and
  Kubernetes manifests no longer have to be typed into an editor (which
  re-indents YAML). Values come from `--set NAME=VALUE`, terminal prompts or
  the form window (`--gui`), and an existing file is replaced only with
  `--force`. A snippet's new `output_file` setting (also in the editor as
  "Write to file") names the file. Twelve starter templates ship built in;
  `wayexpand new` lists them and `--show NAME` prints one to copy.
- Snippets can contain literal `{{` and `}}`, written `{{{{` and `}}}}`, so
  Jinja, Go template and GitHub Actions syntax no longer fails validation as
  an unknown variable. `{{choice:name=A|B}}` gives a drop-down a label, which
  `--set name=...` also uses.
- The Ubuntu PPA builds again. Every Launchpad build since September had
  failed because the daily recipe built plain `main`, which has no vendored
  crates. The Sync Launchpad workflow now republishes the
  `launchpad-vendored` branch on every push to `main`, and that branch now
  includes `debian/`, which `git archive` had been leaving out. The package
  tests also get a private `HOME`, because sbuild's `/sbuild-nonexistent`
  failed the action broker's tests. The recipe must point at
  `launchpad-vendored` (see docs/PACKAGING.md).
- A configuration reload no longer pauses typing. The new configuration
  was read, validated and turned into a matcher on the daemon's event
  loop, which at the 10,000-snippet limit held keystrokes, stop and status
  for about half a second; it is now built on a worker thread while the
  running configuration keeps expanding, and swapped in when ready (about
  5 ms on the loop to start it and 6 ms to swap it at that size). Pause,
  password-field and window state are carried over as of the swap, and a
  change made during a build is built after it.
- The daemon status (and `wayexpand status --json`) reports reliability
  counters and memory use: `output_reconnect_total`,
  `input_reconnect_total`, `input_overflow_total` (evdev input dropped
  because its queue filled), `control_rejected_total` (control requests
  answered busy), and `memory_rss_kib` / `memory_high_water_kib`. The
  fields are additive; the status schema stays 8.
- New benchmarks cover worst-case Unicode buffers (a 4,096-scalar single
  grapheme cluster, zalgo text, ZWJ emoji, regional indicators, Hangul
  jamo) and a full reload (parse, validate, build) at the 10,000-snippet
  limit.
- `wayexpand sync` handles large libraries. Git listings (the tree check,
  the outgoing-history check, staged paths and `sync status`) were read as
  one block capped at 128 KiB, so a library of a few hundred long-named
  `snippets.d` files failed to sync although it was valid. Listings are now
  checked entry by entry, bounded by the configuration model (at most
  10,002 library files, 4 KiB per entry), and the commit takes its paths
  on stdin instead of the argument list.
- A change the GUI could not save is kept instead of dropped. A failed
  write, a configuration lock that stayed busy, or a reload that failed
  after a save conflict used to discard the pending change (settings,
  imports, deletions and toggles are not kept in the editor). The status
  bar now offers Retry save and Discard change; a retry is merged onto the
  current configuration and reports, rather than overwrites, anything that
  was also changed elsewhere, and closing the window warns once first.
- A control-socket request must arrive in full within 2 seconds. The
  timeout applied to each read, so a client sending one byte at a time
  without a newline could hold one of the four control workers
  indefinitely; an oversized request is also refused as soon as it passes
  the 1 KiB limit instead of being read to its end.
- The libei output no longer lets a stream of server events stall typing.
  Server updates are applied before anything is typed, without a limit on
  how many; a bounded batch (512 events, 50 ms) is now applied per attempt,
  and an expansion is refused rather than typed while more are still
  queued.
- A signal no longer cuts a libei wait short. An interrupted poll was
  reported as the deadline expiring, failing the EIS handshake early; it
  now waits out the time that is left.
- The GUI no longer reports a save conflict when the configuration file
  was changed elsewhere to exactly the edit being saved.
- A libei portal session that fails or times out after it was created is
  closed instead of being left open on the shared D-Bus connection.
- The daemon's control socket kept accepting requests after a failed
  accept. One client resetting its connection before it was accepted, or a
  moment without free file descriptors, ended the listener and left
  `status`, `stop` and inserts unanswered until restart. The Action Broker
  no longer spins on accept when out of descriptors.
- A signal arriving during a poll no longer looks like a lost device or
  connection. evdev reopened every keyboard and reset key state, the
  input-method source and the libei output reconnected, and connect-time
  Wayland roundtrips failed, all on EINTR; an interrupted poll now counts
  as "nothing ready yet".
- Backspacing into an expanded replacement could make a word-boundary
  trigger typed before the expansion match again and erase text the user
  had typed (found by the matcher fuzzer). The matcher kept the text that
  preceded a consumed trigger although the replacement now sits between it
  and the cursor; it now drops that context and treats what precedes the
  cursor as unknown, so a word-boundary trigger typed directly against a
  replacement (`signaturebtw`) no longer expands either.
- Reconnecting no longer stalls the daemon. Output and input-method key
  pass-through reconnects (which can wait up to 30 seconds on the portal)
  run on a worker thread, and retry backoff waits on the reactor's waker
  instead of sleeping, so stop, reload, status, and inserts are handled
  throughout.
- The libei output route can reconnect inside a running daemon. Every
  reconnect through the RemoteDesktop portal used to stall for 30 seconds
  and end the daemon: ashpd caches one D-Bus connection per process, and
  its reader ran on a per-session Tokio runtime that closing the first
  session shut down. Portal sessions now share a runtime that lives as long
  as the process, and a session is closed on every exit path.
- The 250-character limit on replacements typed key by key (the libei
  keysym fallback used with xdg-desktop-portal-kde) is now the
  `[settings] keystroke_max_chars` setting (1–5000, default 250), also under
  Settings > Engine in the GUI. The refusal message names the setting.
  Typing takes about 12 ms per character.
- First-run configuration setup creates every missing directory (including
  a missing `~/.config`) with mode 0700. Under Ubuntu's default 0002 user
  umask it created group-writable ancestors and then refused to load the
  configuration it had just written as insecure. The insecure-parent error
  now also says how to fix it.
- The README and Packaging guide point to the experimental prebuilt
  packages, and the German README no longer tells users to install from the
  PPA, which publishes no WayExpand binaries.
- GNOME certification now runs on the IBus route and is optional, like Sway
  and Hyprland; KDE on input-method-v2 is the only production-required
  certification. GNOME's compositor does not offer input-method-v2, so the
  previous required GNOME job could never pass and would have blocked every
  release. IBus evidence can certify GNOME (only GNOME); certified evidence
  records `certification_limitations`: verified but non-atomic replacement,
  no application filters without a window tracker, and no external IME
  composition.
- The Action Broker re-validates an action's program immediately before
  each launch (owner, permissions, parent directories, and that the path
  still resolves to the validated file) instead of only when the policy
  loads, so a change made after loading is refused rather than executed.
- `wayexpand sync` no longer treats a failed remote query (authentication,
  DNS, connectivity) as "the remote has no branch", which skipped fetching
  and verifying the remote before pushing; it reports the redacted Git error.
- Release tarball checks could pass while an archive contained `.git`:
  `grep -q` closed the pipe, `tar` died of SIGPIPE, and `pipefail` turned
  the match into a non-match. Archives are now listed to a file and checked
  there, and the pattern also matches a bare `.git` entry.
- Compositor certification requires online runners only for
  production-required compositors. Optional ones (Sway, Hyprland) run when
  their runner is online instead of failing the preflight, or leaving a job
  queued with no runner and holding a release behind it.
- A running keystroke stream (the libei keysym fallback types one character
  every 12 ms) now stops before the next character when the window tracker
  reports a focus change, or when the daemon abandons the transaction after
  its deadline; cursor keys after the text are checked the same way. The
  result is reported as uncertain and is never retried. Previously the
  checks ran only before typing began, so later characters could reach a
  newly focused window or continue after a reported timeout.
- The core `TextInjector` implementation for `Box` now forwards `admit`,
  `set_output_checkpoint`, and `shutdown` instead of silently using their
  defaults.
- The release installer names each install by version and payload hash and
  reuses an existing directory only when its content matches, so a
  corrected archive with the same version no longer keeps the old binaries.
- GUI saves no longer lose or revert changes: a change queued behind a save
  in flight is replayed onto it by snippet ID (previously it was written
  from a stale snapshot and reverted the earlier save, and a third queued
  change replaced the second). After a revision conflict the change is kept,
  the disk version is reloaded, and the change is merged when it touches
  other snippets; when the same snippet changed on both sides, the external
  version is loaded, the editor keeps the user's draft, and the status says
  it was not saved. A busy configuration lock is retried instead of
  dropping the change.
- The experimental build workflow now builds only the branch or tag it is
  dispatched on, and release runs must name a `vX.Y.Z` tag of this
  repository before any job checks code out. A free-form ref could name a
  fork's pull-request commit and run it with the workflows' cache, release,
  secret, and attestation privileges (CodeQL alert 11).
- The daemon no longer starts one Tokio worker per CPU for D-Bus: zbus's
  blocking API (used by the KWin tracker) built a CPU-sized runtime, which
  took a 16-core KDE session to 31 of the units' 32 allowed tasks and would
  exceed the limit on larger machines. The Action Broker likewise sizes its
  runtime and blocking pool to its unit instead of the CPU count.
- Daemon shutdown no longer hangs when the control socket file has been
  removed (for example by a runtime-directory cleanup); the listener is
  woken through its own socket rather than by connecting to the path.
- **Insertion guarantees.** Keystroke routes (evdev capture with libei or
  wlroots output) no longer erase text they cannot verify unless the user
  opts in with the new `[settings] compatibility_mode`: `tracked-focus`
  (erase only while the KDE window tracker confirms the trigger's exact
  window, checked immediately before typing) or `untracked-focus` (also
  without a tracker). A focus change between matching and typing, or a
  tracker that stops, refuses the expansion in every mode. Refusals happen
  before the first keystroke, are never retried, and are counted.
  Curated packs cannot grant this consent. **Upgrade note:** evdev users must
  set `compatibility_mode` (or use Settings > Engine in the GUI); until then
  `doctor` reports insertion as blocked and typed expansions are refused.
  Verified routes (input-method-v2, IBus) are unchanged.
- Daemon status schema 8 adds `insertion_assurance`
  (`verified`/`compatibility`/`blocked`/`unavailable`), `compatibility_mode`,
  `focus_assurance`, `output_refused_total`, and `output_last_refusal`.
  `doctor` explains the active guarantee and flags a blocked route; the GUI
  toolbar, diagnostics, and Engine settings and the TUI header show whether
  insertion is verified or in compatibility mode, along with refused and
  uncertain expansions. The CLI status contract fixtures now also cover the
  schema 7 `output_*` fields they had missed.
- Added a manually triggered experimental build channel: uncertified x86_64
  and aarch64 archives plus `.deb` and `.rpm` packages from the dispatched branch or tag, with
  checksums and build provenance attestations. Artifacts are labeled with
  their commit and versioned so the next certified release upgrades them;
  publishing a GitHub prerelease is opt-in and never marked latest. Certified
  releases keep the compositor certification gate.
- Added `{{local_date}}`, `{{local_time}}`, and `{{local_datetime}}` (with UTC
  offset), which render in the system time zone and accept the same `+3d`-style
  offsets. `{{date}}`, `{{time}}`, and `{{datetime}}` remain UTC.
- Snippets using a backward date offset such as `{{date-1d}}` no longer fail
  validation; validation rendered templates at the Unix epoch, where the
  offset underflowed.
- The Ansible and Puppet fleet guides no longer restart every account's user
  service on snippet changes (running daemons reload fleet snippets on their
  own). They validate the merged configuration as each listed desktop user,
  restart only after a policy change and only where logind reports a running
  user manager, and deploy `policy.toml` as `0644`; the previous `0600`
  example made the policy unreadable to user services, which then refused
  to start. The self-restarting `:restart-daemon` example snippet is removed.
- An input event that exceeds the per-event expansion budget now treats its
  skipped text as unknown context, so a word-boundary trigger typed next no
  longer matches as if it began the input.
- Per-keystroke matching on long buffers is faster: an NFC quick check
  replaces full-buffer normalization, non-NFC text normalizes only the
  trailing graphemes the longest trigger can use, and Backspace after ASCII
  no longer segments the whole buffer (a full 4096-character buffer drops
  from about 160 µs to 7 µs per keystroke and Backspace).
- `wayexpand backup` now syncs the backup and its directory to disk, verifies
  the copied length, and removes an incomplete backup instead of reporting
  success.
- The snippet picker selects its visible results with a partial sort instead
  of sorting every match on each query change.
- The Indonesian GUI locale is now translated. It previously shared the
  English text for every string, so selecting Bahasa Indonesia showed
  English; a test now rejects shared English/Indonesian string arms.
- On the evdev + libei route, output transactions run on the output worker
  as pending reactor state instead of blocking the reactor. Stop, status,
  and `explain` stay responsive during slow or wedged output; input, focus,
  pause, reload, and further output still wait for the outcome, so ordering
  and undo bookkeeping are unchanged.
- GUI command previews run in a transient systemd user unit with the
  daemon's sandbox directives and minimal environment, so a preview fails
  where the real expansion would. Without a systemd user manager they run
  directly, as before. A test keeps the directives identical to the units.
- Documented that the units' filesystem directives (`ProtectHome=`,
  `ProtectSystem=`, `PrivateTmp=`, `ReadWritePaths=`) are silently skipped
  on hosts that restrict unprivileged user namespaces.
- Daemon status (schema 7) reports output-route health: `output_health`
  (`ok`, `recovering`, `stalled`), a count of transactions with an unknown
  outcome, and the number and age of retired output workers that block
  reconnection. `wayexpand doctor` shows them with restart guidance and
  treats a stalled route as unhealthy.
- `wayexpand insert --verified <trigger>` binds an insert to the window
  focused when the command runs and fails instead of falling back when that
  window cannot be identified exactly. The new organization policy
  `require_verified_insert_target` (safe mode) refuses unverified inserts.
- Git sync no longer accepts outgoing history whose file names contain
  newlines; such a name could pass the library-path check as two allowed
  paths and publish a file that was later deleted.
- Conditional config saves no longer fail with a spurious "busy" error when
  another tool has written legacy (ID-less) snippets back; this is now
  reported as a revision conflict.
- A legacy configuration that WayExpand cannot write (for example a
  root-managed file) now loads with snippet IDs assigned in memory instead
  of failing while trying to persist the migration.
- Espanso imports now support strict semantic filtering and JSON migration
  reports, so entries with unmapped behavior can be discarded before activation.
- Fixed Debian, RPM, and Arch packages pointing their user units at the
  user-installer path `~/.local/lib/wayexpand/current/bin`; packaged units are
  rewritten to `/usr/bin` again.
- Fixed daemon shutdown waiting out the full usage-writer deadline and
  skipping the final usage flush: the control server's clear handle kept the
  writer channel open, so the writer now receives an explicit shutdown request.
- `wayexpand stats --clear` now falls back to a locked local clear when the
  running daemon predates `stats-clear`, instead of reporting success without
  clearing; statistics discarded by a clear are no longer counted as queue
  rejections.
- Git sync diagnostics no longer panic while redacting URLs quoted with
  multi-byte characters (for example `»…«` in localized Git messages).
- The release installer now rolls back the `current` pointer on failure or
  interruption, matching the source installer; both installers remove partial
  staging directories and keep only the active and previous version payloads.
- Window-focus updates now use a bounded latest-snapshot channel, coalescing
  obsolete focus events during bursts while preserving unknown-focus and
  disconnect states instead of allowing an unbounded event backlog.
- Usage-stat clears and daemon flushes now share a bounded interprocess lock,
  preventing a concurrent flush from resurrecting cleared history; temporary
  stats files are created exclusively without following symlinks. When the
  daemon is running, clears are serialized through its writer and discard
  pre-clear queued counters without blocking input processing; a failed file
  clear retains both writer- and reactor-buffered counters for a later retry.
- Added `wayexpand support-bundle`, an allowlisted diagnostic summary that
  excludes configuration paths, snippet contents, credentials, and raw errors;
  optional output files are private and non-overwriting, while diagnostic
  subprocesses have bounded output and teardown.
- User installer version directories now use the built binary's full version
  identity plus a hash of the staged payload, so installs from newer commits or
  changed local assets cannot silently reuse stale files.
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
  symlinked primary config, repairs that file's permissions after remote
  updates, and verifies outgoing tree modes as well as paths.
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
- Git sync lock acquisition now rejects symlinks, special files, foreign
  owners, and hard-linked targets before locking or changing permissions.
- Action Broker audit files are opened without following symlinks and must be
  private, user-owned regular files with a single hard link.
- User installation now restores the previous active version (or removes the
  first-install pointer) when a filesystem error or handled termination signal
  interrupts setup after the atomic version switch.
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
