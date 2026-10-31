# Production Readiness Progress

Updated: 2026-10-31 (America/Denver; latest evidence collected through 2026-10-31 UTC)

## Action Broker caller authorization: 2026-10-31

- Replaced UID-only dispatch authorization with a mandatory polkit check at the
  executor entry point. The broker obtains the connecting process PID from
  `SO_PEERCRED`, reads its `/proc/<pid>/stat` start time, and asks polkit to
  authorize the exact PID/start-time/UID plus configured action ID. Unknown,
  disabled, malformed, or invalid-parameter requests are rejected before a
  prompt. The packaged policy requires fresh active-session authentication;
  missing polkit, policy, or session authorization fails closed. Only one
  authorization prompt runs at a time, with a two-minute deadline and shutdown
  cancellation.
- Added the policy to Debian, RPM, and Arch packages. Release and source user
  installers print the administrator installation requirement. Action IDs are
  restricted to ASCII letters, digits, period, underscore, and hyphen so the
  requested action can be named safely in the polkit prompt. Updated broker
  and threat-model docs to describe the consent gate and its limits.
- Regression evidence: `cargo test --locked -p action-broker` passed (88
  passed, 1 existing ignored sandbox probe; integration: 9 passed, 1 ignored
  test requires interactive polkit consent). The polkit helper success/denial
  path, missing process identity, action ID validation, and PID start-time
  parsing are covered. `cargo clippy --locked -p action-broker --all-targets
  -- -D warnings`, formatting, shell syntax, XML parsing, and
  `scripts/test-action-broker-e2e.sh` passed. The end-to-end test confirms an
  unauthorised request cannot create its action marker.
- Remaining evidence: package builds and hosted CI have not yet run on this
  change. A real desktop authorization prompt has not been exercised. One
  existing SIGTERM integration scenario now requires a live interactive grant
  and is ignored in headless tests; subprocess cancellation remains covered by
  executor tests. Polkit consent is not automatic application identity or an
  OS sandbox, and a user can still approve a deceptive request.

## Live KDE session health and library validation: 2026-10-31

- Built release CLI and daemon binaries from commit `0446fd0ef4ff`; both
  identify that exact commit. `wayexpand validate --fleet --json` passed for
  the active library: 26 expansions, no hotkeys, no command-backed snippets,
  no policy violations, and no trigger-risk findings.
- The read-only release CLI `doctor --json` exited unhealthy for runtime
  readiness, while reporting the configuration valid. The running daemon is
  commit `1fb141d767f7`, status schema 14; current source expects schema 15.
  Its status reports evdev capture with libei keysym fallback, focus-tracked
  best-effort/non-atomic replacement, and no full-Unicode or sensitive-field
  signal. The old status schema also leaves current daemon configuration state
  unavailable to the new CLI. This establishes a stale runtime and unsafe,
  uncertified route; it does not prove the cause of an individual missed
  trigger.
- `systemctl --user show` confirms `wayexpand-evdev.service` is active and
  launches the old versioned daemon. No install, symlink switch, reload, or
  service restart was performed. Those changes affect live keyboard capture
  and require deployment authorization under the project instructions.
- This is live session diagnostics, not compositor certification: no text was
  inserted, and focus, physical-key capture, and client document contents were
  not tested.

## Broker-only command policy: 2026-10-31

- Added `organization.require_broker_actions`. In enforced safe mode, direct
  executable expansion and hotkey commands are rejected by config validation;
  the engine checks again before expansion dispatch, queued hotkey execution,
  and the public direct hotkey execution API. GUI command preview blocks direct
  executables while allowing a named action to proceed to broker handling.
- Added regressions proving a direct expansion configured before policy
  application cannot create a side-effect file, direct hotkeys are blocked at
  both execution boundaries, named actions pass the broker-only check, and
  audit mode does not enforce the setting. Validation tests cover both
  expansion and hotkey configurations; GUI preview has a dedicated policy
  test. Existing Espanso import tests confirm shell executable content is not
  imported as an executable snippet.
- End-to-end tracing found named hotkeys were accepted by policy but sent to
  the direct process launcher, which has no executable path for broker actions.
  Hotkey execution now uses the same broker IPC path as command expansions,
  disables output capture, and reduces broker failures to a stable error
  category. A fake broker regression exercises both direct and queued-worker
  hotkey execution and verifies the action ID, empty parameters,
  non-inherited environment, and disabled stdout capture.
- Added the field to CLI config-schema and doctor JSON/human diagnostics, and
  documented the safe-mode setting and the reviewed-import-to-broker workflow.
- Verification: the full locked workspace suite and workspace Clippy passed
  after the broker-routing fix. After extending the regression through the
  queued worker, `cargo test --locked -p wayexpand-core` passed with 481 tests
  passed and 2 ignored; workspace Clippy, formatting, and `git diff --check`
  passed. The fake broker verified both request paths. No managed broker
  action was executed as part of these tests.
- Limitation: this policy requires use of the Action Broker, whose executable
  allowlist and service sandbox remain its current shared security boundary.
  It does not add per-action Landlock/seccomp isolation or protect against
  malicious same-UID processes. The policy is root-owned administrator policy
  when applied by the daemon; a user-editable library cannot strengthen the
  daemon's administrator policy.
- The policy-only changes were pushed as `3523331a`, the evidence record as
  `a92064d6`, and the broker-hotkey routing fix as `1b37d426`, all over SSH
  using `Stephan Loesevitz <stephan.loesevitz@gmail.com>`. GitHub CI run
  `41224310749` was active on `0446fd0e` at the time this follow-up was
  prepared. The asynchronous worker regression and live-session evidence are
  separate uncommitted follow-up changes and need CI on their own commit.

## Exact-head KWin output smoke: 2026-10-31

- Rebuilt and ran `scripts/test-doctor-certify.sh` on clean commit
  `e54af8492ef4`; its isolated KWin 6.6.6 report contract passed. Preserved
  the report at
  [`docs/evidence/doctor-certify-e54af849.json`](docs/evidence/doctor-certify-e54af849.json).
- Simple replacement, word-boundary delimiter, cursor placement, multiline
  output, focus switching and cancellation, target closure, command completion,
  configuration reload, daemon restart, refusal of unsupported Unicode, and
  refusal of an unconsented keystroke route passed. Build identity matched.
  The libei keysym fallback measured 20 injector transactions per size: p95
  12.715 ms (small), 780.415 ms (medium), and 3.126 s (large).
- Capture was stdin-simulated. Physical input, overlap during non-atomic
  replacement, keyboard hotplug during output, Unicode insertion, password
  fields, and suspend/resume remain untested. The report correctly says
  `overall: partial` and `certified: false`; this does not close the P0 gate.
- Fresh local supply-chain checks passed: `cargo audit --deny yanked` found no
  vulnerabilities or yanked locked dependencies, with the three already
  allowed unmaintained zbus 5-lineage advisories; `cargo deny check` passed.
  GitHub still has zero online certification runners. Upstream CI, Launchpad,
  CodeQL, and fuzz jobs pass for `753365dc`, but do not cover this local branch,
  now 11 commits ahead of `origin/main`.

## Validated configuration snapshot reuse: 2026-10-31

- `ExpansionEngine` now reads parsed app filters, form fields, hotkeys, undo
  chord, and effective-trigger indexes directly from its retained immutable
  `ValidatedConfig`. It no longer deep-clones these metadata collections into
  engine-local copies during construction. Added/updated coverage verifies the
  engine uses the validated filters, chords, form definitions, and trigger
  metadata.
- `cargo test --locked -p wayexpand-core`: 469 passed, 0 failed, 2 ignored;
  core Clippy with warnings denied passed. Full serial workspace tests passed
  across all test binaries (only tests marked ignored remained ignored), and
  workspace Clippy with warnings denied, formatting, and `git diff --check`
  passed.
- A same-host Criterion comparison for amplified 10,000-snippet engine
  construction measured 455 ms before and 505 ms after, with overlapping
  intervals and no statistically detected change (`p = 0.09`). This supports
  no latency claim; the intended benefit is lower duplicated engine metadata
  and a single authoritative immutable source. Real desktop certification
  remains open.

## Hotplug during replacement certification coverage

- Added a distinct `keyboard-hotplug-during-nonatomic-replacement` required
  scenario. It requires physical device removal or reconnection during paced
  output and verifies that the daemon interrupts further synthetic input.
- The private KWin driver explicitly returns `UNVERIFIED` because its capture
  is stdin-simulated and it has no physical keyboard devices. The isolated
  `doctor --certify` report also labels the check `not-tested`; existing
  inotify and cancellation unit tests do not count as desktop evidence.
- Updated matrix uniqueness/count checks, safety categorization, support-bundle
  expectations, and production acceptance documentation. Focused shell
  contracts passed (`test-certification-matrix.sh`, KWin driver contract,
  full certification-driver contract, support-bundle contract), as did the
  17 `doctor::live_certification` unit tests, CLI Clippy with warnings denied,
  formatting, documentation contracts, and `git diff --check`. The full
  `test-doctor-certify.sh` run was stopped after it entered its private KWin
  harness (which has a 16-minute deadline); this environment has no physical
  keyboard session for the required hotplug scenario. No production
  certification claim is made.

## Hotplug cancellation with buffered evdev events

- Follow-up source review found that the output monitor returned early to
  translate retained raw events without checking the inotify watch. This could
  delay hotplug interruption while a large non-key event backlog was drained.
- The output-monitor path now polls the topology descriptor nonblocking before
  each buffered event slice and fails closed if the descriptor is invalidated.
  A regression creates a device-node notification while 512 non-key raw events
  are buffered and verifies cancellation is reported before the raw backlog is
  consumed.
- `cargo test --locked -p wayexpand-backend-evdev`: 25 passed, 1 ignored;
  `cargo clippy --locked -p wayexpand-backend-evdev --all-targets -- -D
  warnings`: passed; daemon physical-input cancellation regression passed.
  This proves monitor ordering with test devices, not physical hotplug during
  real output. The release certification requirement remains open.
- Exact source commit `97b6c105`: `RUST_TEST_THREADS=1 cargo test --workspace
  --locked --quiet` passed with 1,370 passed, 0 failed, and 6 ignored;
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, formatting,
  documentation contracts, Markdown links, and diff checks passed.
- A fresh read-only `wayexpand doctor --json` against the active desktop shows
  KDE Wayland, an older running evdev + libei daemon, keysym-fallback output,
  non-atomic replacement, and no sensitive-field signal. The input-method-v2
  protocol and IBus route are unavailable in this session. Doctor correctly
  exits unhealthy for production readiness; this is not evidence that the
  configured personal snippets themselves are invalid or that physical
  replacement passed.

## Current verification and certification runner status: 2026-10-31 UTC

- Refreshed GitHub state and incorporated upstream `main` through
  `753365dc4d7f`. That upstream revision includes the per-event profile and
  batched-randomness optimizations (`314037b5`) and the KWin tracker-ID rename.
  Its CI run `41173643747`, Launchpad sync `41173643741`, CodeQL run
  `41173643603`, and scheduled fuzz run `41189987642` all completed
  successfully; package smoke jobs for Ubuntu 26.04 and Fedora also passed.
  The repository has no open GitHub issues, and its Actions runner API still
  reports zero certification runners. These results are for upstream
  `753365dc`, not for the local commits below.
- Rebased the three local verified commits onto that upstream tip without a
  merge commit. Exact combined HEAD `7ca29928`: `RUST_TEST_THREADS=1 cargo
  test --workspace --locked --quiet` passed with 1,369 tests passed, 0 failed,
  and 6 ignored. Workspace Clippy with `-D warnings`, formatting, and diff
  checks passed. This exact combined HEAD has not run in GitHub Actions because
  it has not been pushed.
- Closed a gap in the non-atomic evdev output monitor: it now watches
  `/dev/input` topology events through inotify in addition to polling already
  open keyboards. Any device-node change interrupts output immediately, and
  device discovery is deferred to the next normal input turn so it cannot
  delay cancellation. Watch creation precedes initial discovery to avoid a
  startup race. If the watcher fails during operation, the source reports a
  retryable error so pending output is treated as uncertain and canceled.
- Added regressions for inotify node creation and for a hotplug event
  interrupting output before discovery. The watcher now parses inotify event
  records and fails closed on invalidation, unmount, read failure, or a drain
  budget overrun; regressions cover node creation and invalidation. Full evdev
  tests: 24 passed, 1 ignored. `RUST_TEST_THREADS=1 cargo test --workspace
  --locked --quiet`: 1,367 passed, 0 failed, 6 ignored. Workspace Clippy with
  `-D warnings`, formatting, and `git diff --check` passed. This is unit
  evidence; physical hotplug during real output remains in the open
  certification matrix.
- Fixed a misleading live/offline snippet explanation: when no sensitive-focus
  event is active, it now reports that field safety is unverified instead of
  claiming “not a sensitive field.” A positive signal still blocks the
  expansion. This makes the explanation accurate on routes such as evdev that
  cannot identify password fields; it does not add sensitive-field detection
  or change route enforcement.
- Added a core regression proving the no-signal explanation is informational,
  explicitly says safety is not verified, and does not hide the independent
  “would expand” result. The focused explanation suite passed (8 tests), then
  `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet` completed with
  1,364 passed, 0 failed, and 6 ignored. Workspace Clippy with `-D warnings`,
  formatting, and `git diff --check` passed.
- Re-ran `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet` on
  synchronized commit `6665d98401b5b610a11dece5851c53b8b5fff709`; every
  executed workspace test passed. The run included 2 ignored tests in their
  ordinary harnesses.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check` passed.
- Re-ran the physical-input interruption regressions: both evdev output
  monitor tests, the daemon cancellation/drop-queued-work test, and the short
  polling cancellation test passed. These exercise mocked devices/channels;
  they do not establish physical keyboard correctness in a real client.
- Ran the ignored live systemd preview integrations against the active user
  manager. `live_preview_runs_in_the_daemon_sandbox` passed, verifying actual
  `NoNewPrivs` and seccomp state, bounded timeout cancellation, and transient
  unit cleanup. `live_preview_respects_custom_xdg_paths` also passed and
  removed its temporary test tree. This validates command-preview sandboxing
  on this host, not the separately open per-action broker device-namespace
  gate.
- GitHub's repository Actions runner API reported `total_count: 0`. The
  compositor certification workflow therefore has no registered self-hosted
  runner available. CI run
  [41163212844](https://github.com/cyberducttape/wayexpand/actions/runs/41163212844)
  for this commit was still pending with jobs queued at the time of the check;
  this is not a pass or failure result.
- The certification gate remains open. A real supported desktop session with
  physical capture and actual target-client verification is still required;
  no software-only test result substitutes for that evidence.

## Exact-head local verification: 5ab177d8 / 2026-10-31 UTC

- `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet`: 1,363 passed,
  0 failed, 7 ignored across the workspace.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- The commit contains documentation and evidence only; the tested Rust source
  is the same clean source revision `fc063a4054ab` identified in the KWin
  evidence summary.
- No production certification claim follows from these desktop-independent
  tests. Physical capture integrity, compositor acceptance, and long soak
  gates remain open.

## Current-source KWin doctor smoke evidence: 2026-10-30

- Ran `cargo run --locked -p wayexpand -- doctor --certify --json` from clean
  commit `494aac93b15d` in an isolated private headless KWin 6.6.6 session.
  Preserved the machine-readable report at
  `docs/evidence/doctor-certify-494aac93.json`.
- The report records 13 passed checks, 4 not-tested checks, and one measured
  backend-latency check. The route negotiated libei keysym fallback; capture
  was simulated over stdin. It remains `partial` and `certified: false`.
- Twenty injector samples per size reported p50/p95/p99 of
  12.569/12.765/12.828 ms (small), 779.039/787.663/807.576 ms (medium), and
  3,123.171/3,159.319/3,159.673 ms (large). These exclude physical capture
  and client paint latency. Physical capture, concurrent input integrity,
  Unicode on this keysym route, suspend/resume, password-field behavior, and
  long soak remain unverified.
- Updated `docs/CERTIFICATION.md` and `DEVELOPMENT_PLAN.md` to identify this as
  the latest report without promoting it to production certification.
- Queried the active KDE desktop with the installed CLI's read-only doctor
  command. It exited unhealthy and reports a live evdev + libei keysym-fallback
  route, focus-tracked best-effort non-atomic replacement, no full-Unicode
  support, and no sensitive-field signal. The daemon is connected, but this
  route remains uncertified; the JSON is in `/tmp/wayexpand-active-doctor.json`
  and was not added to the repository because it describes this machine's
  private runtime state.
- A newer read-only check using the working-tree CLI found the active daemon
  at commit `1fb141d767f7` with control schema 14; the working-tree CLI is
  `fc063a4054ab` and requires schema 15. This makes that CLI's live explanation
  unavailable. It does not establish a mismatch with the user's installed
  GUI/CLI or prove the cause of an expansion failure. The saved library
  validates with 26 expansions; `.ty` and `.ssl` are enabled and render
  successfully, while `.test` is absent. No service restart or install was
  performed.

## Focused KWin real-client matrix: 2026-10-30 local / 2026-10-31 UTC

- Ran the selected seven-scenario matrix across five real target clients in a
  private headless KWin 6.6.6 session on clean source commit
  `fc063a4054aba7069474cc1237fdc93ea767eae1`. All 35 cells passed.
- The store and its metadata summary are preserved at
  `docs/evidence/kwin-real-client-fc063a4054ab.txt` and
  `docs/evidence/kwin-real-client-fc063a4054ab.summary.json`. The summary binds
  the run to source/binary hashes, KWin and kernel versions, GTK/Qt/client
  versions, and the `us` layout.
- Verified multiline output and continued typing, cross-window app filtering,
  cancellation after focus change, target closure without late output into a
  second window, config reload, daemon restart, and refusal of unsupported
  Unicode while preserving the trigger.
- The harness used stdin-simulated capture and libei keysym output. It does not
  verify physical-input overlap, password fields, layout changes, suspend/resume,
  or end-to-end latency. This is partial compatibility evidence, not route
  certification. The prior non-atomic overlap corruption remains a P0 gate.
- `scripts/test-kwin-certification-driver.sh`: passed.
- The preserved store was independently loaded through
  `scripts/kwin-certification-driver.sh config-reload`; it returned the stored
  GTK client result as `pass` for the matching CLI build and KWin version.
- Focused GUI diagnostics tests: 4 passed. The Wayland dashboard is already
  implemented in the diagnostics view and distinguishes live route reports
  from application testing and production certification.
- GitHub run `41165906032` (Launchpad sync) completed successfully. Main CI
  run `41165905971` remained queued, while Push on main run `41165906042` was
  in progress on the last check. The repository still had no self-hosted
  certification runners.
- The JSON evidence validator, local Markdown links (54 files, 493 links),
  documentation contracts, and `git diff --check` passed.

## Wayland compatibility dashboard overview: 2026-10-30

- Added a concise overview at the top of GUI diagnostics with detected
  compositor, active capture and injection route, capture readiness, Unicode
  insertion report, replacement assurance, external IME composition, focus
  visibility, sensitive-field capability, compositor permission health,
  matcher/output p99, and recent failed expansion count.
- Capability labels distinguish route-reported support, route-reported limits,
  unsupported-on-route, and unreported evidence. Empty latency windows display
  as no samples rather than `0 µs`; failure count is shown only when the daemon
  actually supplied its recent-outcome field. A prominent note distinguishes
  live reports from application tests and production certification.
- Added accessible UI regression coverage for reported support, unsupported
  IME composition, fallback diagnostics, metrics, and recent failure counts;
  added coverage that absent capability/latency data remains unreported.
- `cargo test --locked -p wayexpand-gui`: 175 passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check`: passed.

## Performance verification: 2026-10-30

- Confirmed the daemon owns and reuses pending/ready vectors on its normal
  event-processing paths. Safety-gated output transfers ownership into a
  transaction batch, so reusing that storage would require redesigning output
  transaction ownership; no wrapper-only refactor was added.
- Confirmed `ExpansionEngine` already retains Unicode normalization scratch
  (`raw`, normalized scalars, and source offsets) across matching calls. The
  warmed scalar API remains stack encoded and has no recurring allocations on
  ordinary ASCII input.
- `RUST_TEST_THREADS=1 cargo test --workspace --locked`: passed across all
  workspace crates.
- `cargo bench --locked -p wayexpand-core --bench matcher` and
  `cargo bench --locked -p wayexpand-core --bench template_render`: completed
  on Linux x86_64 (kernel 7.0.0-38-generic), Rust 1.96.0, 16 logical CPUs.
  Selected absolute observations: matcher latency was ~4.0 µs at 100 entries,
  ~6.8 µs at 1,000, and ~8.9 µs at 10,000; amplified construction was ~30 ms
  at 1,000 and ~223 ms at 10,000. Near-miss Unicode matching was ~4.0–4.7 µs.
  Criterion reported regressions against its local saved baseline for several
  workloads, including amplified construction and template rendering. The
  baseline's source revision and host conditions are not recorded here, so
  these are follow-up profiling signals rather than attributable regressions.
- `cargo run --locked --release -p wayexpand-core --example
  input_path_allocations`: 0 allocations over 100,000 warmed immediate ASCII
  events and 0 over 100,000 warmed deferred ASCII events.
- These microbenchmarks do not measure backend capture/injection or certified
  desktop end-to-end latency. Reproducible comparisons need pinned hardware,
  controlled load, and a named source baseline.

## Evdev signal-test isolation and scalar API audit: 2026-10-30

- Moved SIGUSR1 disposition changes into a dedicated ignored helper test that
  is launched by the parent regression test as a separate process. This
  prevents parallel tests in the unit-test process from observing the
  temporary handler; the helper still verifies restoration before exit.
- Audited production scalar paths: evdev and input-method-v2 enqueue
  `TextChar`, while IBus calls `process_deferred_char`. Owned `Text(String)` is
  retained where a protocol delivers multi-character commits/composition.
- Made latency recorder tests use private metric windows and counters. This
  removes scheduling-dependent assertions against process-global telemetry,
  while retaining explicit tests that contended recording returns immediately
  and increments dropped-sample counters.
- `cargo test --locked -p wayexpand-backend-evdev -- --nocapture`: 21 tests
  passed; the subprocess-only helper is intentionally ignored by the parent
  test runner and was executed successfully through its parent test.
- `cargo test --workspace --locked --quiet`: passed across the workspace under
  default parallel execution; two tests were ignored in the core suite and
  three subprocess/platform helpers were ignored by their parent runners.
- `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, local
  Markdown link checks (54 files, 483 links), documentation contracts, and
  `git diff --check`: passed.

## Replacement safety and current-source smoke evidence: 2026-10-30

- Re-audited capture/injection gates, deferred-output cancellation, transaction
  outcome handling, and GUI/status assurance labels. Existing code refuses
  unverified erasure by default, rechecks focus at paced output checkpoints,
  stops non-atomic output on observed physical input, and discards uncertain
  results rather than retrying. Regression tests cover these cases in the
  daemon, core engine, and assurance wrapper.
- Ran the current-source isolated KWin test-client harness on clean commit
  `589d2682360f` and preserved the report at
  `docs/evidence/doctor-certify-589d2682360f.json`. Twelve output/focus/reload/
  refusal checks passed and latency was measured; the report remains `partial`
  because capture was simulated and physical keyboard, password-field,
  suspend/resume, Unicode on this keysym-only mode, and non-atomic concurrent
  input cases were not certified.
- A live `wayexpand doctor --json` check also showed this host's active route
  as evdev plus libei keysym fallback: focus-tracked best effort, no
  sensitive-field signal, and no atomic replacement. This is expected to
  remain a release blocker unless a supported protocol route is available and
  certified in real client sessions.
- Validated the saved report as JSON and refreshed the certification guide's
  latest-run link and latency summary. Markdown links and documentation
  contracts passed after the update.

## Process supervisor lifecycle audit: 2026-10-30

- Added Linux regression tests for an already-exited child at supervisor
  initialization, group cleanup of a descendant that ignores SIGTERM, and
  inherited stdout closure after group cleanup. Added a controlled test proving
  that `setsid()` descendants escape process-group cleanup, then kills the
  escaped test process explicitly to avoid leaking it.
- Exercised and verified the EPERM cleanup-failure accounting path. This is a
  deterministic classification test, not a privileged cross-UID `kill(2)` test.
- Clarified the operational boundary: the broker's service cgroup is a final
  cleanup boundary on service stop, not a per-action cgroup for action timeout.
- `cargo test --locked -p wayexpand-process-supervisor -- --test-threads=1`:
  10 tests passed. Afterward, `RUST_TEST_THREADS=1 cargo test --workspace
  --locked --quiet`, workspace Clippy with `-D warnings`, formatting,
  documentation links (54 files, 483 local links), documentation contracts,
  and `git diff --check` all passed.

## Dependency updates, CI identity gate, and documentation sprint

- Main CI failed at `Documentation Validation` after Dependabot PRs #31, #34
  and #35 were merged in the web UI: the push-range identity check rejected
  the bot commits and the owner's web merge commits. Commit `e86ad188`
  accepts both only with GitHub's pinned web-flow signature; its CI run
  passed. Branch protection is still not enabled (see
  [`docs/REPOSITORY_GOVERNANCE.md`](docs/REPOSITORY_GOVERNANCE.md)).
- PR #33 (eframe 0.37) failed to compile. Commit `fa4b52e7` ported the GUI,
  restored the bundled symbol fonts egui 0.37 moved behind a feature, and was
  merged as `f0df8d91`. PR #32 (notify 8.2.1) was rebased by Dependabot after
  a lockfile conflict and is the only open pull request.
- `wayexpand doctor | head` panicked with exit 101; commit `2427f55c` makes a
  closed stdout exit 1.
- Documentation sprint: removed `docs/archive/`, 11 unreferenced and
  superseded `doctor --certify` reports (all `certified: false`), and one
  superseded KWin overlap-reproduction report; corrected the
  contributing guide's MSRV (1.92, not 1.95) and project layout; added the
  missing `profile` and pack commands to the man page and `wayexpand help`;
  completed the configuration-limits table; replaced unsupported performance
  expectations in troubleshooting with measured values and live metrics;
  brought the German README up to date.

## Current-source verification and remaining release gates

- On current `main` commit `e7f2c013d2caa3124d577f69010fcf0b55ae62af`,
  GitHub CI [41155252763](https://github.com/cyberducttape/wayexpand/actions/runs/41155252763)
  passed all 33 jobs. This includes locked debug/release workspace tests,
  debug/release Clippy, matcher construction and allocation checks, CLI/UI/GUI
  smoke checks, installer/system tests, Debian/RPM builds and install smoke
  tests on Ubuntu 26.04 and Fedora, security audit, clean-source build, and
  headless KWin/Sway tests. The main-branch workflow
  [41155252450](https://github.com/cyberducttape/wayexpand/actions/runs/41155252450)
  passed all four jobs, and Launchpad sync
  [41155252845](https://github.com/cyberducttape/wayexpand/actions/runs/41155252845)
  passed. These results establish CI and package evidence only; headless tests
  do not establish physical-input or final-target-text integrity.
- GitHub CI run [41076472952](https://github.com/cyberducttape/wayexpand/actions/runs/41076472952)
  passed all 33 jobs on `65d4c7c3`, including locked debug/release workspace
  tests, debug/release Clippy, integration and system tests, security audit,
  clean-source build/test, Debian/RPM artifacts, Ubuntu 26.04 and Fedora
  package-install smoke tests, and headless KWin/Sway E2E. Launchpad sync
  [41076473005](https://github.com/cyberducttape/wayexpand/actions/runs/41076473005)
  also passed.
- Fuzz run [41040557794](https://github.com/cyberducttape/wayexpand/actions/runs/41040557794)
  passed all eight targets on ancestor commit `dda02daf`. This predates the
  eframe 0.37 GUI port, so it is not fuzz evidence for that later change.
- On this checkout, all 131 CLI binary tests pass; workspace formatting,
  targeted CLI Clippy, 54 Markdown files / 483 local links, and documentation
  contracts pass. The CLI help test now asserts the pack diff/rollback and
  per-app profile commands are listed.
- GitHub still has no online self-hosted certification runners. The only
  repository ruleset is `pro` with enforcement disabled; `main` is unprotected.
  PR #32 (notify 8.2.1) is open with green checks. No current CI result or
  headless desktop test satisfies physical-input, final-target-text, or
  production compositor certification requirements.

## Current-source nightly fuzz run

- Ran every target in `docs/FUZZING.md` for 60 seconds on clean source commit
  `750d44f8c3fe`: `matcher` (48,172 executions), `config_parse` (836,481),
  `broker_frame` (3,616,061), `espanso_import` (390,078), `pack_import`
  (149,922), `app_filter` (538,411), `ibus_keys` (642,720), and
  `engine_sequence` (46,249). All eight exited successfully with no crash or
  sanitizer finding (6,268,094 executions total).
- Fuzz builds refreshed `fuzz/Cargo.lock` to include
  `wayexpand-process-supervisor`, now a transitive dependency of the core
  library. Retained this reproducibility update. Cargo nightly also emitted
  non-fatal redundant `homepage` metadata warnings.
- The ignored `fuzz/corpus/` directories now contain locally generated
  coverage corpus data; no crash artifacts were produced. The 60-second runs
  do not replace the scheduled 20-minute fuzz jobs.

## Main-branch governance gap

- GitHub returned `404 Branch not protected` for the public repository's
  default branch. Open issues are empty; five Dependabot pull requests remain
  open. The production release workflow already gates tagged artifacts on
  compositor certification, but that does not prevent an unchecked merge to
  `main`.
- Added [`docs/REPOSITORY_GOVERNANCE.md`](docs/REPOSITORY_GOVERNANCE.md) with
  the observed state, exact required CI job contexts, branch ruleset criteria,
  release-gate distinction, and verification commands. The production
  acceptance checklist now tracks applying and testing the ruleset as open.
- GitHub repository settings were not changed. A repository administrator
  must apply the ruleset and verify that a failing required check blocks a
  merge; the documentation is not enforcement.

## Current-source private KWin regression run

- Re-ran `scripts/test-kwin-e2e.sh` from clean source commit `5e0a434d2f10`
  on KWin 6.6.6. All 15 output, focus, lifecycle, and default-refusal
  scenarios passed. The captured report is
  [`docs/evidence/kwin-e2e-5e0a434d2f10.txt`](docs/evidence/kwin-e2e-5e0a434d2f10.txt).
- The libei keysym fallback produced 20 measured transactions per size bucket:
  p95 was 13.007 ms (small), 782.196 ms (medium), and 3.141 s (large).
- The harness used stdin-simulated capture. It does not test physical input
  during output, so it cannot close the overlap-corruption or production
  certification gates.
- The corresponding local `scripts/test-sway-e2e.sh` could not start because
  `sway` is not installed in this environment. The previous GitHub Sway run
  predates the current local changes, so current-source Sway E2E remains
  unverified here.

## Current-source isolated daemon soak

- Ran `SOAK_SECONDS=180` with control probes every loop and a controlled daemon
  restart every 60 seconds from clean source commit `750d44f8c3fe`. All 295
  rounds passed, including two restarts; the preserved measurements are in
  [`docs/evidence/daemon-soak-750d44f8.txt`](docs/evidence/daemon-soak-750d44f8.txt).
- RSS peaked at 20,144 KiB after warmup, with no FD or thread growth (14 FDs,
  15 threads throughout sampled points). Status p95 was 22.585 ms; explain p95
  was 38.003 ms. Two restart samples measured 614.384–696.084 ms.
- This synthetic, output-disabled soak does not replace the 72-hour
  compositor-session test.

## Current remote CI and dependency-update audit

- GitHub `main` CI run [41020746599](https://github.com/cyberducttape/wayexpand/actions/runs/41020746599)
  passed on remote commit `f6c338b42f40`, including Rust tests, native aarch64,
  Debian/RPM artifacts, and package install/reinstall/remove smoke tests on
  Ubuntu 26.04 and Fedora latest. This remote commit predates the current local
  changes; it is not evidence for those unpushed changes.
- Four open Dependabot PR runs (tokio, rustix, notify, and read-fonts) passed
  Rust, build, security, and package jobs; each failed only at
  `Documentation Validation / Verify commit identity policy`. The local fix
  is commit `ebe78f22` and `bash scripts/test-commit-identity.sh` passes here.
  The fix is absent from remote `main`, so the PR workflow cannot use it yet.
- The open `eframe 0.37.0` PR has additional real failures in workspace tests,
  clean builds, aarch64, Debian packaging, GUI concurrency stress, and Rust
  1.92 MSRV. Do not accept that update as a routine dependency bump; it needs
  a separate API/MSRV migration.
- GitHub reports zero online self-hosted runners. No current CI or runner
  evidence closes the real compositor certification gate. No remote branch,
  PR, or workflow was changed during this audit.

## Independent full-workspace release verification

- Re-ran `RUST_TEST_THREADS=1 cargo test --release --workspace --locked` on
  the current checkout. The command completed with exit code 0; every executed
  unit, integration, and documentation test passed. Environment-dependent and
  manual-display tests remained ignored by their existing annotations.
- `cargo clippy --release --workspace --all-targets --locked -- -D warnings`
  passed, and the release allocation probe reported zero allocations across
  100,000 warmed ASCII events for each immediate and deferred core path.
- The source-level safety review confirmed that non-atomic output watches for
  physical evdev activity, cancels further simulated output, marks the
  transaction uncertain, and never replays it. This is containment only: the
  documented private-KWin overlap reproduction still demonstrates partial
  text corruption, so production replacement integrity remains an open P0
  gate pending an atomic route or attended physical-capture evidence.

## Live session and external CI audit

- Read-only checks in the active KDE Wayland session found the installed
  daemon at `27e83eace764`, while the locally built doctor identifies as
  `0a77cbb887d4` and this source checkout has advanced further. The connected
  route is explicit-consent evdev capture with libei keysym fallback at 12 ms
  pacing. Status reports focus-tracked best-effort replacement, no atomic
  replacement, no sensitive-field signal, no layout awareness, and no full
  Unicode guarantee. `wayexpand doctor --json` exits 1 and reports no safe
  automatic setup. No service or desktop state was changed.
- The status endpoint showed 21 persisted expansion events and no recent
  failures. That is observational runtime telemetry, not evidence of correct
  final text or safe behavior in password fields. Production certification
  remains blocked.
- GitHub currently reports no open issues and zero registered self-hosted
  runners. Five open Dependabot PRs have failing CI: four only fail the
  commit-identity check because the remote base workflow predates local commit
  `ebe78f22` (the exact Dependabot tuple exception); that script's positive,
  negative, and spoofed-identity tests pass locally. The `eframe 0.37.0` PR
  also has actual workspace/API and Rust 1.92 compatibility failures and
  requires a deliberate GUI migration before it can be accepted.
- The local branch is ahead of `origin/main`. Per the release mission, these
  fixes and diagnostics have not been pushed; remote PR checks
  cannot reflect them until an authorized push updates the base workflow.
- Ran `scripts/test-kwin-e2e.sh` on clean source commit `0acd0ed87ce6` in its
  isolated private KWin 6.6.6 session. All 15 output, focus, lifecycle, and
  default-deny scenarios passed. With the 12 ms keysym fallback, p95 was
  12.985 ms (small), 783.759 ms (medium), and 3.142 s (large). The report is
  preserved in [`docs/evidence/kwin-e2e-0acd0ed87ce6.txt`](docs/evidence/kwin-e2e-0acd0ed87ce6.txt)
  and indexed by [`docs/CERTIFICATION_MATRIX.md`](docs/CERTIFICATION_MATRIX.md).
  Capture remained stdin-simulated, so this does not close the real-capture
  P0 gate.

## Current-source release installation checks

- `scripts/test-install-release.sh` passed, including invalid option/config
  preflight, atomic activation, rollback after injected failures, and safe
  uninstall. Its fixture redirects `HOME`, XDG paths, runtime directory, and
  the session bus into temporary storage.
- `scripts/test-install-user.sh` passed against the current checkout. It
  built release binaries and exercised custom XDG roots, spaces/systemd path
  escaping, configuration preservation, immutable payload checks, version
  switching, invalid-config upgrade refusal, and uninstall protections.
- `scripts/test-release.sh` passed: deterministic source archive generation,
  repeatable binary archive assembly, checksum verification, invalid
  `SOURCE_DATE_EPOCH` rejection, required payload checks, and packaged CLI/UI
  smoke tests.
- These are local x86_64 source/archive checks. No container runtime is
  available here, so actual Debian/RPM package-manager install/reinstall/remove
  tests on supported distribution images remain unverified. The active
  desktop installation was not changed.
- Hardened the package smoke test guard: package install/reinstall/remove now
  requires both its explicit disposable-test opt-in and a Docker/Podman
  runtime marker. Added a host-side regression that verifies the opt-in alone
  is refused before package-manager commands. The guard test,
  ShellCheck, shell syntax, and CI workflow actionlint all passed. GitHub's
  documented container-job example checks for `/.dockerenv`, matching the
  marker used by this workflow.

## Fast-path refactor verification

- Audited the proposed borrowed matcher construction, delimiter processing,
  matcher telemetry, and reusable result-storage patterns. All four are
  already implemented in the current source: `Matcher::try_new` accepts
  `AsRef<str>`, delimiter events encode into a stack buffer and use the shared
  text scanner, matcher samples use `try_lock` with a dropped-sample counter,
  and immediate/deferred processing expose caller-owned reusable result APIs.
- Existing core regressions cover equivalence between scalar/borrowed and
  owned text processing, delimiter behavior, and result-vector capacity reuse.
  No duplicate implementation was added.
- Fresh verification: `RUST_TEST_THREADS=1 cargo test --release --workspace
  --locked` passed with exit code 0; `cargo clippy --release --workspace
  --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check`, and
  `git diff --check` passed.
- Those checks establish correctness for the current snapshot. Follow-up
  allocation and benchmark measurements are recorded below.

## Fast-path allocation and benchmark follow-up

- Expanded and re-ran the release allocator probe: zero allocations across
  100,000 warmed nonmatching ASCII events in each of the immediate and
  deferred core paths. The isolated matcher memory probe built
  10,000-snippet libraries in 15.18 ms (plain), 93.00 ms (80,000 amplified
  triggers), and 129.93 ms (90,000 Unicode triggers); measured steady heap was
  2.1, 7.6, and 9.6 MiB respectively. This is isolated matcher construction,
  not process RSS or full config reload.
- Improved `matcher_latency` benchmark validity: it now feeds scalar events
  using `process_text_char_into` and a reused result vector, removing temporary
  per-character strings from the timed harness. Because this changes the
  workload, historical matcher latency numbers are not directly comparable.
- Repeated short matcher runs reported 5.45–6.77 µs at 100 snippets,
  4.22–4.67 µs at 1,000, and 4.35–4.37 µs at 10,000. The source-render
  template case measured 8.16 µs, while precompiled rendering measured
  3.82 µs; the latter aligns with the prior interleaved measurement. Full
  matcher and template results, host details, and limits are in
  [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).
- These are short, host-specific benchmark results with run-to-run spread.
  They establish measurements, not end-to-end latency targets or a release
  performance guarantee. No matcher rewrite was justified by this evidence.

## Current-source private KWin certification run

- Ran `cargo run --locked -p wayexpand --bin wayexpand -- doctor --certify
  --json` from clean commit `c94a4f634bfb`. Preserved the machine-readable
  report at
  [`docs/evidence/doctor-certify-c94a4f634bfb.json`](docs/evidence/doctor-certify-c94a4f634bfb.json).
- The private KWin 6.6.6 run passed simple replacement, word-boundary
  delimiter, cursor placement, multiline output, focus switching, focus-change
  cancellation, target closure, command completion, reload, daemon restart,
  and unsupported-character refusal. Build identity matched the source commit.
- The negotiated route was libei keysym fallback at 12 ms key pacing. Injector
  p95 was 15.1 ms for small, 783 ms for medium, and 3.14 s for large
  replacements, with 20 samples per size. These measurements exclude physical
  capture and client paint latency.
- The result is explicitly partial and `certified: false`. Unicode insertion
  was not tested on this keysym-only route. Physical keyboard capture,
  password-field behavior, suspend/resume, and long soak remain untested. This
  adds current-source output/focus evidence but does not satisfy the P0 release
  gate.
- Re-ran `scripts/test-kwin-e2e.sh --keep
  second-trigger-during-nonatomic-replacement` against clean commit
  `0136588a3eda`. It failed as expected: 124/128 replacement characters
  remained, concurrent input corrupted the output, and the stale second
  trigger expanded. The report is
  [`docs/evidence/kwin-overlap-reproduction-0136588a.json`](docs/evidence/kwin-overlap-reproduction-0136588a.json).
  Since this harness simulates capture over stdin, this reproduces the
  non-atomic output race but does not test the physical evdev interruption
  monitor. The monitor cannot make an already non-exclusive route atomic.
- Attempted the private KWin input-method-v2 route with
  `scripts/test-kwin-e2e.sh --source=input-method --keep`. All four dedicated
  insertion/password/form scenarios were unverified because KWin 6.6.6 did not
  expose an input-method-v2 manager/seat in this session. The availability
  result is
  [`docs/evidence/kwin-input-method-unavailable-0136588a.json`](docs/evidence/kwin-input-method-unavailable-0136588a.json);
  it does not establish availability on other KWin configurations.

## GUI large-library benchmark

- Ran `cargo bench --locked -p wayexpand-gui --bench gui_edit --
  --sample-size 10 --warm-up-time 1 --measurement-time 2` on the current
  source. Conflict lookup stayed near 28 ns and draft edit plus preview near
  15 µs from 100 through 10,000 snippets. Full save increased from 3.19 ms to
  375 ms; it runs on the bounded GUI background worker. Hardware, toolchain,
  full results, and limits are recorded in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).
- The benchmark completed successfully. Criterion warned that the full-save
  samples exceeded the requested two-second measurement target and extended
  sampling; these figures are directional, not release latency SLO evidence.

## End-to-end default-deny insertion check

- Added `keystroke-route-refuses-without-consent` to the private KWin and Sway
  E2E suites and to `doctor --certify`. With compatibility consent off, it
  requires the connected non-atomic route to report `blocked`, records a
  compatibility-mode refusal, leaves `;safe` unchanged in the actual Wayland
  sink, and records zero uncertain output. The doctor report stays incomplete
  if this check is missing.
- The focused private KWin scenario passed on commit `b2037049`. The full
  `scripts/test-doctor-certify.sh target/debug/wayexpand` contract also passed
  on that clean commit, including the new check. All 17 focused CLI
  certification unit tests passed; CLI Clippy, workspace formatting,
  ShellCheck, shell syntax, documentation contracts, Markdown links, and
  whitespace checks passed.
- This verifies the default-deny behavior against an isolated client. It does
  not verify physical keyboard capture, the evdev interruption monitor, or
  eliminate the documented corruption when compatibility consent is enabled.

## Per-action device namespace feasibility

- Confirmed the systemd contract in the official
  [`systemd.exec(5)` documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#PrivateDevices=):
  `PrivateDevices=` was added in v239 and implicitly enables `PrivateUsers=`
  for a user-manager service, requiring unprivileged user namespaces.
- Tested a transient user unit on this systemd 259 environment. It failed
  before `ExecStart` with `218/CAPABILITIES`; `unshare -Ur true` also failed
  with `Operation not permitted`. This container cannot validate the private
  device namespace. The packaged broker remains unchanged; enabling the
  directive based on syntax validation alone would risk making the service
  fail at startup on restricted hosts.

## Certification evidence identity and reproducibility

- The scenario evidence collector emits schema 4 with the running kernel
  release and SHA-256 of the exact WayExpand CLI used for doctor/status probes.
  A report cannot set `certified: true` without a complete version map for
  every target application, including application version and toolkit name
  and version. The certification workflow validates this metadata before
  running the full scenario matrix and passes it into the report.
- Regression coverage verifies the CLI hash, kernel field, exact client
  metadata, rejection of absent/incomplete maps, and the existing fail-closed
  route/scenario checks.
- Fixed a certification gate defect discovered during review: the collector
  compared daemon control `status_schema` (14) with the CLI status JSON
  contract's `schema_version` (10), so it rejected every current daemon as
  stale. The contract now carries a distinct `daemon_control_schema`; a
  regression runs the production-capable Sway input-method path through the
  collector using schema 14 and verifies it passes the live status probe.
- Verification passed: `scripts/test-certification.sh`,
  `scripts/test-certification-driver.sh`,
  `scripts/test-certification-matrix.sh`,
  `scripts/test-certification-report.sh`, `shellcheck scripts/*.sh`, workflow
  YAML parsing, the workflow's version-map jq validation, and `git diff
  --check`. `actionlint` is unavailable in this environment; the workflow was
  parsed as YAML but not checked with actionlint.
- After separating the status schema versions,
  `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand-daemon` passed 206
  tests and `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand --bin
  wayexpand` passed 128 tests. The daemon suite also confirms the bounded
  status response still fits its control frame.
- The production release gate still fails closed because no route is
  production-certified. This change strengthens report provenance; it does
  not create real-session certification evidence. The Actions API currently
  reports zero registered self-hosted runners for this repository.

## Matcher and event-path optimization review

- The proposed borrowed matcher constructor is already implemented:
  `Matcher::try_new<I, S>` accepts any `S: AsRef<str>` and stores only trie
  indexes and edges. Regression coverage builds from borrowed `String` values
  and borrowed string slices.
- Delimiters already use stack-backed UTF-8 encoding and the common text
  scanner, with no recursive event dispatch. The engine also exposes reusable
  immediate and deferred result-vector APIs; owned results do not borrow from
  the reusable vectors.
- Matcher telemetry captures elapsed time before recording, uses `try_lock`
  on the input path, and exports a dropped-sample counter. Its regression test
  exercises lock contention and the counter.
- Fresh verification: `RUST_TEST_THREADS=1 cargo test --workspace --locked`
  and `cargo clippy --workspace --all-targets --locked -- -D warnings` both
  passed. `cargo fmt --all -- --check` passed.
- Criterion matcher and template-render benches completed. Representative
  results versus the stored local baseline: matcher lookup was 4.28 µs (100
  triggers, no significant change), 4.42 µs (1,000, improved ~5%), and 4.55
  µs (10,000, improved ~3%). Amplified construction of 1,000 / 5,000 / 10,000
  triggers measured 29.8 / 66.2 / 162.2 ms, each improved versus baseline.
  Template built-ins measured 4.26 µs (improved ~7%); precompiled built-ins
  3.76 µs (within noise); 1 MiB full render 0.943 ms (within noise); bounded
  preview 0.304 ms (improved ~8%). The 4,096-character NFD type/backspace
  case measured 53.0 µs (~2% slower), and some Unicode near-miss cases showed
  small regressions. These are host-specific Criterion comparisons, not
  end-to-end input latency or allocation-count measurements; investigate the
  Unicode deltas before claiming a general performance improvement.

## Deterministic binary release archive assembly

- Updated `scripts/package-release-binaries.sh` to normalize staging umask,
  archive path order, mtime (`SOURCE_DATE_EPOCH`, default `0`), and numeric
  ownership before gzip. Invalid timestamp values are rejected.
- Extended `scripts/test-release.sh` to build the same release payload under
  umasks `077` and `002` and compare the resulting tarballs byte-for-byte.
  The complete release smoke test passed: archive equality, checksum, payload
  inspection, and packaged CLI checks all succeeded. This proves deterministic
  archive assembly on the same host, not independent compiler reproducibility.

## Owner-thread startup cancellation integration test

- Added a regression that invokes the production `LibeiInjector` startup path
  against a temporary Unix EIS socket. The test server accepts the connection
  and stays silent; cancellation is issued only after accept. The caller
  receives `Cancelled` within one second, and the owner thread returns through
  the readiness channel and is joined. The test restores `LIBEI_SOCKET` after
  completion and passed alongside the backend suite with eight test threads.
- Verification: `cargo test --locked -p wayexpand-backend-libei --
  --test-threads=8` passed all 45 tests; backend Clippy with `-D warnings` and
  `cargo check --workspace --locked` passed. This validates startup
  cancellation against a local silent server, not portal/compositor failure or
  the 72-hour desktop lifecycle gate.

## Workspace verification after lifecycle changes

- `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` passed all
  executed workspace test targets. The core suite passed 465 tests (2 ignored),
  the daemon 206, the GUI 173, and the CLI 128; four environment-dependent or
  manual tests across the workspace were ignored. No test failed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed.
  The workspace includes the new direct EIS cancellation/handshake tests and
  the current source changes; this remains automated evidence, not a live
  physical-input certification.

## Daemon fail-closed regression suite

- `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand-daemon` passed all
  206 tests. Coverage includes refusal before unverified erasure, focus-change
  refusal both before and during output, stop-on-interruption without replay,
  uncertain-output preservation across reconnect, stale command-result
  rejection, and status/health reporting.
- Re-ran the complete daemon suite on commit `0df037d0`: 206 passed, zero
  failed. A read-only active-session diagnostic reported evdev + libei keysym
  fallback, non-atomic best-effort replacement, and no sensitive-field signal;
  this is compatibility evidence only, not production certification.
- These are code-level regressions only. The private KWin smoke run still
  cannot test physical keyboard interruption during output; attended desktop
  target-text evidence remains a P0 acceptance requirement.
- On current source `54ea2a6b`, reran two core preflight/journal tests and five
  daemon interruption cases: physical input cancels non-atomic output without
  replay, focus changes stop partial output and queued batches, and timeout is
  recorded as uncertain. All seven passed. The production gate script also
  returned its expected blocked status because no route is production-required
  and certified; the private KWin JSON remains partial compatibility evidence.

## Action Broker security regression suite

- On commit `e956cbb8`, `cargo test --locked -p action-broker` passed all 80
  library tests, 7 broker binary tests, and 10 integration tests. Coverage
  includes launching the pinned executable inode after pathname replacement,
  Landlock read/write denial, required-audit failure handling, continuous
  stdout/stderr deadlines, child cancellation, peer credential sizing, and
  concurrent socket ownership.
- `cargo clippy --locked -p action-broker --all-targets -- -D warnings`
  passed. This validates the tested Linux host paths; remaining per-action
  isolation scope is tracked in the P2 plan item below.

## Per-action broker IP-socket restriction

- Added `sandbox.deny_network` for Linux action profiles. The child installs
  a seccomp filter after `no_new_privs` and Landlock: IPv4, IPv6, packet, and
  netlink socket creation plus `io_uring_setup` fail with `EPERM`; AF_UNIX
  remains available. A native-architecture guard rejects ABI changes, and
  unsupported architectures fail before spawning. The executor-level test
  launches a child under the actual configured profile and verifies each
  behavior.
- Verification on x86_64: `cargo test --locked -p action-broker` passed 83
  library tests (1 child-only ignored), 7 broker binary tests, and 10
  integration tests; package Clippy with `-D warnings`, workspace `cargo
  check --locked`, documentation contracts, and Markdown links passed.
- The full `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` run also
  passed on this worktree with no failures. `cargo fmt --all -- --check` and
  workspace compilation passed after the API change.
- Cross-compilation with `cargo check --locked -p action-broker --tests
  --target aarch64-unknown-linux-gnu` passed. This checks AArch64 code and test
  compilation, not seccomp behavior on an AArch64 kernel.
- This filter prevents network socket creation; it is not complete network
  containment if a local AF_UNIX peer passes a network descriptor. The
  packaged service retains its global network denial. Device isolation and
  cross-architecture runtime verification remain open.

## Restricted per-action device-access profile

- Added opt-in `sandbox.deny_devices`. With the Landlock allowlist, the child
  is denied `ioctl`, `recvmsg`/`recvmmsg` (preventing Unix-socket FD receipt),
  and `mknod`/`mknodat`; `log_output` is rejected because it could inherit a
  terminal descriptor. Existing sandbox profiles default this option off.
- The executor-level probe exercises the combined network and device seccomp
  policy after spawn, verifying IP-family socket denial, AF_UNIX socket
  availability, `io_uring_setup`, `ioctl`, and `recvmsg` behavior. Config tests
  cover TOML parsing, compatibility defaults, and the inherited-output rule.
- This is a restricted device-access profile, not full device isolation:
  there is no private device namespace, and a device node nested inside an
  explicitly granted directory is not independently identified by type. The
  systemd broker unit still needs `PrivateDevices=yes` (or an equivalent
  per-action namespace) before WayExpand can claim complete device isolation.
- Verification on x86_64: `cargo fmt --all -- --check`, all 84 broker library
  tests (1 child-only ignored), 7 broker binary tests, 10 integration tests,
  and broker Clippy with `-D warnings` passed. The executor regression verifies
  `/dev/zero` cannot be opened and the configured syscall restrictions take
  effect after exec.
- `cargo check --locked --workspace` passed. AArch64 test cross-compilation
  initially found that Linux AArch64 has no standalone `mknod` syscall; the
  filter now uses `mknodat` on AArch64 and RISC-V, and both the x86_64 host and
  AArch64 cross-check pass. Runtime behavior on AArch64 remains unverified.
- Documentation contracts passed, and the Markdown checker validated 76 files
  and 523 local links.

## systemd private-device feasibility check

- Reviewed versioned systemd execution docs: systemd 249 says
  `PrivateDevices=` is unsupported in per-user service managers, systemd 253
  permits it for user services when `PrivateUsers=` is enabled, and current
  docs describe implicitly enabling `PrivateUsers=` for that case. Sources:
  [systemd 249 `systemd.exec`](https://www.freedesktop.org/software/systemd/man/249/systemd.exec.html),
  [systemd 253 `systemd.exec`](https://www.freedesktop.org/software/systemd/man/253/systemd.exec.html),
  [current `systemd.exec`](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html).
- Tested both `PrivateDevices=yes` alone and with `PrivateUsers=yes` using
  transient user units on local systemd 259. Both failed before exec with
  status `218/CAPABILITIES`; the user-manager journal reported
  `Failed to drop capabilities: Operation not permitted`. This environment
  cannot validate the device namespace, so it does not justify enabling the
  directive unconditionally in the shipped user unit.
- No unit directive was added. The per-action restricted device profile
  remains defense in depth; complete device isolation needs a successful
  runtime test on an eligible supported host and a documented
  systemd/user-namespace minimum.
- Revalidated on the active user manager (systemd 259) on 2026-10-31: a
  transient `PrivateDevices=yes` one-shot unit again exited before exec with
  `218/CAPABILITIES`; `unshare -Ur true` failed with `Operation not permitted`.
  The transient unit was collected. This confirms the local environment still
  cannot produce the required device-namespace evidence.

## Per-user installer transaction verification

- `scripts/test-install-user.sh` passed against the current worktree. It built
  the release payload and exercised temporary-home installs, preflight option
  and config validation, atomic `current` switching, rollback on a post-switch
  filesystem failure and SIGTERM, immutable payload integrity, a second build
  identity, preserved user config, custom XDG state/config roots, and paths
  containing spaces/systemd specifiers. The test uses a temporary home and
  isolated runtime bus; it did not enable or restart the active desktop service.
- This is installer-script evidence. Package-manager upgrade/rollback and
  uninstall behavior on supported distributions, signed artifact provenance,
  and a real user-service upgrade rehearsal remain open release criteria.

## Release archive installer verification

- `scripts/test-install-release.sh` passed. Its hermetic archive fixture
  exercised install and uninstall paths, option validation, evdev-consent
  gating, atomic rollback after service restart/enable failures, corrected
  same-version payload handling, XDG service drop-ins, and user configuration
  preservation. `systemctl` and process checks were stubbed; no host service or
  package manager was changed.
- This validates the release archive installer script against a synthetic
  layout, not a published signed archive or package-manager install/upgrade on
  a supported distribution.

## Release binary archive smoke test

- `scripts/test-release.sh` passed on the current x86_64 host. It built the
  release binaries, produced the distributable tarball and SHA-256 file,
  verified the checksum, checked required packaged helpers/assets and file
  modes, exercised packaged CLI commands, and ran the isolated Wayland harness
  far enough to validate its adjacent helper lookup. The script also passed
  its fresh-config smoke check.
- This validates the x86_64 release archive build path locally. It does not
  validate arm64 execution on this host, published signatures/attestations,
  distro package-manager upgrades, or production compositor certification.

## Direct EIS connection and handshake failure coverage

- Extracted the stream-to-handshake step used by session startup and added an
  integration-style test using a real Unix listener. The server accepts the
  direct socket but sends no EIS handshake data; startup returns the expected
  deadline error within one second, and the server thread is released and
  joined.
- Combined with the in-flight connection cancellation regression, this
  covers pending connect cancellation and a stalled accepted EIS socket. It
  does not emulate all owner-thread/device negotiation behavior or a live
  compositor. Verification passed: all 44 libei backend tests, backend Clippy
  with `-D warnings`, `cargo check --workspace --locked`, formatting, and
  whitespace checks.

## Direct EIS in-flight cancellation regression

- Factored the direct Unix-socket connection race into a generic future helper
  used by the production path. Added a deterministic pending-future test that
  cancels during connection, verifies the attempt future is dropped, and
  enforces a one-second upper bound. This covers the in-flight case without
  depending on nondeterministic Unix listen-backlog saturation.
- `cargo test --locked -p wayexpand-backend-libei` passed all 43 tests;
  focused connection tests and `cargo clippy --locked -p
  wayexpand-backend-libei --all-targets -- -D warnings` passed. This tests the
  cancellation race contract; it does not emulate a stalled live compositor.

## Current-source isolated KWin smoke report

- Built commit `54ea2a6b` and ran `wayexpand doctor --certify --json` in its
  isolated private KWin 6.6.6 session. The preserved report
  [`docs/evidence/doctor-certify-54ea2a6bb7b4.json`](docs/evidence/doctor-certify-54ea2a6bb7b4.json)
  confirms matching CLI/daemon identity and a clean tested source tree. Twelve
  checks passed; focus change/cancellation, target closure, reload/restart,
  actual client text, refusal before deletion, and build identity were covered.
- The negotiated path was libei keysym fallback with stdin-simulated capture.
  Twenty transactions per size measured p50/p95/p99 of 12,596/12,744/12,833
  µs (small), 777,117/778,028/778,690 µs (medium), and
  3,121,038/3,126,440/3,128,708 µs (large). These timings exclude physical
  capture and client paint; 257-byte insertion p95 was about 3.13 seconds.
- Physical keyboard capture, a second physical trigger during non-atomic
  replacement, Unicode insertion on keysym fallback, and suspend/resume were
  not tested. Password-field handling and long-term stability remain
  uncertified. The active desktop daemon was not changed. This report is
  compatibility evidence and does not pass the production certification gate.

## Zero-allocation input fast-path verification

- Re-ran the release allocator probe against commit `f6ed0267`: 100,000
  warmed, nonmatching deferred `TextChar` events produced zero heap
  allocations. This remains a core-path measurement; it does not include
  backend decoding, queueing, or expansion-result construction.
- Confirmed the requested fast-path patterns are already implemented:
  `Matcher::try_new` accepts borrowed and owned strings; delimiter events use
  a stack UTF-8 buffer and share the text scanner; matcher telemetry records
  with `try_lock` and counts samples dropped under contention; and the daemon
  reuses caller-owned pending-result storage.
- Fresh verification: `cargo test --locked -p wayexpand-core` passed (465
  passed, 2 ignored, plus documentation and snapshot integration tests).
  Focused matcher-construction, delimiter-parity, and daemon telemetry
  contention tests passed. `cargo fmt --all -- --check` and `git diff --check`
  passed.
- The release allocation probe reported zero allocations across 100,000
  warmed nonmatching ASCII deferred-input events. This measures the warmed
  core event path only, not XKB lookup or full daemon/backend processing.

## Cancellable direct EIS socket startup

- Replaced the blocking `UnixStream::connect` used by explicit `LIBEI_SOCKET`
  routes with Tokio's asynchronous Unix socket connection. It now observes
  the shared cancellation token and a five-second deadline before handing the
  connected stream to the existing nonblocking EIS handshake.
- Added coverage for preexisting cancellation, missing socket errors, and a
  successful connection converted back to a standard Unix stream. The
  `wayexpand-backend-libei` suite passed all 42 tests with 8 test threads;
  its warnings-as-errors Clippy check and the daemon's bounded-shutdown
  regression passed. Formatting and whitespace checks passed.
- The live portal path and real compositor behavior are unchanged; no
  production certification or physical-input evidence is implied by these
  socket-level tests.
- Pushed as part of `624f5fd6` to `origin/main`. CI run
  [41018444819](https://github.com/cyberducttape/wayexpand/actions/runs/41018444819),
  Fuzz [41018444870](https://github.com/cyberducttape/wayexpand/actions/runs/41018444870),
  CodeQL [41018444375](https://github.com/cyberducttape/wayexpand/actions/runs/41018444375),
  and Launchpad sync
  [41018444904](https://github.com/cyberducttape/wayexpand/actions/runs/41018444904)
  all completed successfully for this commit. CI included debug/release tests,
  Clippy, package builds and install smoke tests, headless KWin/Sway E2E, and
  the daemon soak. None of those jobs exercise physical keyboard capture in an
  attended desktop session.

## Libei owner-loop failure handling

- Added a private session-operation boundary so the thread-affine command loop
  can be exercised without moving real libei objects across threads. A worker
  regression injects an operation error, verifies the exact error is replied
  to the caller, then confirms a following request still receives a successful
  reply and the owner loop shuts down cleanly.
- Extracted the readiness handoff so tests cover a reported startup
  cancellation and an owner thread that exits before reporting readiness;
  both errors return after the ended thread is joined. `cargo test -p
  wayexpand-backend-libei --locked -- --test-threads=8` passed all 39 tests.
  Backend Clippy with `-D warnings`, formatting, and diff checks passed.
- The daemon regression `shutdown_does_not_join_a_worker_stuck_in_backend_teardown`
  passed, confirming the outer output actor returns promptly when a backend
  shutdown blocks.
- This verifies request/reply behavior after an operation failure. It does not
  prove bounded startup when the EIS connection itself never reports
  readiness, or recovery from a non-responsive live compositor; those remain
  lifecycle test gaps.

## Latest verified change: scalar input event fast path

- Added `InputEvent::TextChar(char)` and routed ordinary single-scalar XKB
  actions from the evdev and input-method backends through the core without
  constructing a temporary owned `String`. Multi-scalar text and composed
  commits retain the owned text path. Input-method local composition still
  receives the UTF-8 commit before the scalar event is queued for matching.
- Added immediate/deferred parity coverage, exercised the variant in fuzz
  input generation and the allocation probe, and updated benchmark and
  development-plan scope notes. The probe measures the warmed core event path;
  it excludes XKB lookup and the rest of the backend/daemon pipeline.
- Verification on this working tree: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, and
  `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` passed. Core had
  464 passing tests and 2 ignored; daemon had 206 passing tests; GUI had 173;
  evdev had 21 passing and 1 ignored; the remaining workspace suites also
  passed. The release allocation probe reported zero allocations across
  100,000 warmed nonmatching ASCII `TextChar` events.
- This does not establish zero allocations for XKB lookup or certify real
  desktop capture/insertion. No production compositor route is certified; the
  live KDE route still has the limitations recorded below.
- Committed as `045dfdbb` (`Avoid owned strings for scalar input events`),
  followed by the Unicode regression and progress updates; the complete series
  was pushed to `origin/main` as `624f5fd6`. The active user service was not
  deployed or restarted.
- Updated the core's event-by-event Unicode trigger helper to emit
  `InputEvent::TextChar`, so canonical normalization and grapheme deletion
  cases exercise the same event shape as real scalar key input. Added a
  focused regression for decomposed-accent matching and backspace after
  scalar-by-scalar input. Reverification passed: workspace Clippy,
  `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` (465 core tests
  passed, 2 ignored; 206 daemon; 173 GUI; 128 CLI; all other suites passed),
  formatting, and `git diff --check`.

## Non-atomic interruption health regression

- Strengthened `observed_physical_input_cancels_output_and_drops_queued_work`
  to assert that detected physical input during a simulated non-atomic
  replacement increments the externally reported uncertain-transaction
  counter. The existing assertions also verify that output stops after the
  erase step and queued work is not sent.
- Verification: the focused daemon regression passed with
  `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand-daemon
  observed_physical_input_cancels_output_and_drops_queued_work`; formatting and
  daemon Clippy with `-D warnings` passed.
- This is automated containment evidence only. Real evdev interruption and
  final target-document integrity still require an attended desktop test; the
  P0 certification gate remains open.

## Dependabot commit identity CI failure

- Investigated failed CI run
  [41029636518](https://github.com/cyberducttape/wayexpand/actions/runs/41029636518).
  Its Rust tests, Clippy, aarch64 build, package smoke tests, and other jobs
  passed; documentation validation failed because the commit identity checker
  rejected Dependabot's standard author and GitHub committer identities.
- Added an explicit `--allow-dependabot` checker mode for the exact observed
  author/committer tuple. CI uses it only when the event is a pull request whose
  GitHub-reported author is `dependabot[bot]`; pushes and other PRs remain under
  the original strict owner identity policy. Expanded the isolated script test
  to cover two accepted Dependabot commits, rejection without opt-in, rejection
  of a spoofed bot email, owner identity, and missing refs.
- Verification passed: `bash scripts/test-commit-identity.sh`, the identity
  check on this commit, ShellCheck on both changed scripts, the Launchpad
  workflow safety contract, formatting, and `git diff --check`. The updated CI
  workflow also passed `go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.11
  .github/workflows/ci.yml`.
- This fixes the observed CI policy failure. The correction has not been pushed
  or rerun on GitHub Actions.

## Fail closed when the evdev interruption monitor is missing

- `watches_non_atomic_output_for_input()` previously skipped its monitor when
  evdev mode was enabled, the evdev source was absent, and connection status
  still said `connected`. The monitor's missing-source path already knows to
  cancel and mark output uncertain, but the old predicate could bypass it.
  Pending non-atomic evdev output now always requires the monitor; a missing
  source cancels at the next output checkpoint.
- Added a regression with the source and test monitor absent but connection
  status still connected. It verifies the transaction stops after the erase
  phase and increments the uncertainty counter.
- Verification: all 13 `pending_output::tests` passed; formatting, daemon
  Clippy with `-D warnings`, and `git diff --check` passed. A fresh
  `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` then passed across
  the workspace (core 465 passed / 2 ignored, daemon 207, CLI 129, GUI 173,
  and all other unit, integration, and documentation tests passed), followed
  by workspace Clippy with `-D warnings` and formatting. This is synthetic
  containment evidence and does not replace real evdev plus target-client
  certification.

Follow-up default-parallel testing exposed that exact `+1` assertions on the
process-wide uncertainty counter race with other output tests. Changed those
assertions to require a monotonic increase while retaining the assertions
that this transaction stops after erase and does not insert. All 13 tests
pass with both default parallel scheduling and `RUST_TEST_THREADS=1`; the full
207-test daemon suite also passed with default parallel scheduling.

## Current-source private KWin latency run

- Ran `scripts/test-kwin-e2e.sh output-latency` on clean commit
  `0a77cbb887d4`. The isolated KWin 6.6.6 scenario passed 60 insertions (20
  each at 1, 64, and 257 characters) and verified the final sink text.
- Measured p95 injector latency was 12.997 ms, 782.969 ms, and 3,142.759 ms
  respectively. The source and caveats are recorded in `docs/BENCHMARKS.md`.
  This run used stdin-simulated capture; it does not verify physical input,
  client paint latency, concurrent typing integrity, or an alternative pacing
  interval.

## GUI preview security documentation correction

- Corrected `SECURITY.md`, which still claimed GUI command previews never used
  the daemon sandbox. The implementation and operations guide show that direct
  previews use a transient systemd unit with the capture service directives
  when available; local advanced previews run with desktop-user privileges,
  safe mode blocks those when no service sandbox exists, and previews are not
  included in the Action Broker audit.
- Verification: documentation contracts, local Markdown links, and
  `git diff --check` passed.

## Completed in this work period

- Generalized `Matcher::try_new` to accept `IntoIterator<Item = S>` where
  `S: AsRef<str>`. The builder borrows each item only while inserting its
  characters, and still supports owned `String` iterators. Added coverage for
  `&String`, `&str`, and owned `String`; all 9 matcher tests passed.
- After the API change, `RUST_TEST_THREADS=1 cargo test --workspace --locked
  -q` passed: 460 core tests (2 ignored), 206 daemon, 173 GUI, 128 CLI, and
  all remaining workspace suites passed. Workspace Clippy with `-D warnings`,
  formatting, and diff checks passed.

- Added caller-owned reusable result APIs to `ExpansionEngine` and wired the
  daemon's deferred text paths through a retained pending-result vector. The
  dispatcher drains fully owned results before the vector is reused; regression
  tests compare behavior and verify capacity retention. Matcher construction
  now borrows validated trigger slices, avoiding intermediate trigger clones.
- D verification: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, and
  `RUST_TEST_THREADS=1 cargo test --workspace --locked` passed. The full
  workspace includes 462 core tests (2 ignored), 206 daemon tests, and all
  other unit, integration, and documentation suites.
- Requested release benchmarks completed: matcher lookups measured 4.34–4.83
  µs across 100–10,000 snippets; the 10,000-entry amplified construction case
  measured 175.9 ms in Criterion. The allocation example measured 9.05 ms and
  2.1 MiB steady / 9.4 MiB peak for 10,000 plain triggers, 54.14 ms and
  7.6 MiB / 28.9 MiB for 80,000 amplified triggers, and 81.54 ms and
  9.6 MiB / 41.5 MiB for 90,000 Unicode triggers. Template render benchmarks
  also completed (builtins 4.67 µs; precompiled builtins 3.82 µs; depth-8
  include 485 ns; 1 MiB render 980 µs; bounded preview 345 µs).
  Criterion comparisons are host-local and should not be treated as portable
  release guarantees. The allocation example measures trie construction after
  source trigger strings are prepared; it does not measure per-keystroke
  allocations.
- Ran the source-built `wayexpand doctor --certify --json` against private
  KWin 6.6.6 and preserved the clean-build report at
  [`docs/evidence/doctor-certify-60315df9bf4c.json`](docs/evidence/doctor-certify-60315df9bf4c.json).
  Twelve insertion, focus, reload, restart, refusal, and build-identity checks
  passed. Capture was simulated over stdin; overlap, Unicode output on the
  negotiated keysym fallback, suspend/resume, and physical keyboard capture
  remain untested. This is partial compatibility evidence and does not close
  P0. Injector p95 was 12.712 ms small, 778.047 ms medium, and 3,126.553 ms
  large, excluding capture and client paint.
- The certification report contract, matrix, production-gate, driver, process
  cleanup, and Markdown-link checks all passed; the link checker validated 76
  Markdown files and 519 local links. The gate correctly continues to reject
  production certification because no matrix route is eligible. CI run
  `41006119736` completed successfully on `2614bc53`. The current pushed
  commit `624f5fd6` also passed CI, Fuzz, CodeQL, and Launchpad sync; see the
  run records at the top of this file.
- Added single-character engine entry points that encode into stack storage.
  IBus now avoids constructing a temporary `String` for decoded printable
  characters, and the GUI playground reuses immediate-result capacity while
  replaying characters. A core regression compares deferred character events
  with equivalent text events. Workspace formatting, Clippy, and the full
  locked test suite passed after this change. A release-mode allocation probe
  measured zero allocations across 100,000 warmed deferred nonmatching
  ASCII events. This is a core-engine-only measurement and excludes backend
  decoding, event queueing, and allocations when a match produces output. The
  probe is now part of the locked release CI job as a regression gate.

- Made matcher latency recording nonblocking: capture operation duration
  before metrics synchronization, use `try_lock`, and count skipped samples
  atomically. Added `matcher_latency_dropped_total` to daemon status and bumped
  the status contract to schema 14. A lock-contention regression confirms
  measurement returns without waiting and increments the dropped count.
- Removed delimiter `String` construction and recursive event dispatch by
  stack-encoding the scalar and passing borrowed UTF-8 to the shared scanner.
  A regression compares results and retained matcher state with normal text
  input.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-core -p wayexpand-daemon
  --locked`: passed; 459 core tests passed (2 ignored), and 206 daemon tests
  passed, plus integration and documentation tests. The daemon status
  contract test verifies the new field and schema.
- The schema bump initially exposed stale schema-13 fixtures in GUI and CLI
  contract tests. Updated current-status fixtures and the CLI status-shape
  contract. `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` then
  passed across the workspace; 459 core, 206 daemon, 173 GUI, 128 CLI, and all
  other crate suites passed (four tests ignored).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check` passed.
- A read-only `wayexpand doctor --json` against the active KDE Wayland session
  found that the installed daemon is commit `27e83eace764` with status schema
  13, while this checkout is `2614bc530f1c` with schema 14. Doctor reports the
  runtime as unhealthy and production readiness unavailable. The connected
  route uses evdev capture plus libei keysym fallback; it reports no atomic
  replacement, Unicode guarantee, sensitive-field detection, or layout
  awareness. `wayexpand-evdev.service` is active from the versioned install at
  `~/.local/lib/wayexpand/current` and runs `--source=evdev --backend=libei
  --allow-evdev-sensitive-fields`; the installed CLI and daemon both identify
  as commit `27e83eace764`. The stdin test harness is inactive. The daemon was
  left running and unchanged; updating/restarting a live desktop service is a
  deployment action requiring authorization.
- Fresh verification on source commit `2614bc53`: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, and
  `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` all passed. The
  release allocation probe measured zero allocations across 100,000 warmed
  deferred ASCII events. These checks do not certify physical capture or the
  installed older daemon.
- `bash scripts/test-production-certification-gate.sh` passed, confirming that
  release publication remains blocked while no compositor route is eligible.
- After the signal-restoration changes, the final `RUST_TEST_THREADS=1 cargo
  test --workspace --locked -q` run passed: 459 core tests (2 ignored), 206
  daemon tests, 173 GUI tests, 128 CLI tests, and all other workspace suites
  passed. Workspace Clippy, formatting, and whitespace checks passed again.
- Fixed process-global signal handler leakage in both the evdev poll and
  libei handshake interruption tests. Scoped guards restore the captured
  action even during unwinding; assertions verify the restored handler and
  public flags while tolerating Linux's private `SA_RESTORER` bit. Both
  focused signal tests pass. Reviewed the libei connection lifecycle: setup
  observes cancellation between bounded operations, and the daemon regression
  confirms an abandoned attempt drops a late injector instead of installing
  it.

- Changed case propagation to accept `Cow<str>` and return the original
  borrowed/owned string for lower or mixed case, avoiding the unnecessary copy
  of already-rendered text. Upper/title transformations still create owned
  output. Pointer-identity regression tests cover both Cow variants.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-core --locked -q`: 458 passed,
  0 failed, 2 ignored, plus core integration and documentation tests.
- `cargo clippy -p wayexpand-core --all-targets --locked -- -D warnings` and
  the focused case-propagation tests passed. Workspace Clippy, formatting,
  and `git diff --check` passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed.
- `cargo bench -p wayexpand-core --locked` completed. Representative results
  on this host: matcher latency 5.1–10.8 µs for 100–10,000 snippets;
  4,096-scalar combining-cluster type/backspace 115–130 µs; Unicode near-miss
  matching 4.2–5.7 µs; matcher construction 36 ms / 87 ms / 361 ms for
  1,000 / 5,000 / 10,000 snippets; reload parse/validate/build 80 ms and
  384 ms for 1,000 and 10,000 snippets. These are a single host run, not a
  before/after comparison, and do not measure full-path allocation counts.
- Criterion flagged the existing pathological `unicode_cluster_spans_buffer`
  case at 129–297 µs as slower than its stored baseline (reported +94% to
  +333%). It exercises a 4,096-scalar grapheme fallback and is unrelated to
  the Cow case-propagation edit. Treat this as a benchmark regression to
  reproduce and investigate separately; the shared host showed substantial
  outliers. Cow pointer-identity tests establish that unchanged borrowed and
  owned values retain their buffers, but full input-event allocation
  instrumentation remains outstanding.

- Removed the case-classification `Vec<char>` allocation and multiple scans;
  alphabetic case flags are now collected in one pass.
- Changed Unicode backspace to segment a bounded 64-scalar suffix and fall
  back to the full retained history only if the cursor needs more context.
  UTF-8 byte length is maintained as the buffer changes, and a regression
  covers a grapheme spanning the suffix plus long regional-indicator parity.
- Reused evdev event-vector capacity, replaced per-batch path copies with
  active device-slot identities, and capped translation at 128 raw events per
  turn while retaining unprocessed events for subsequent turns. Added a
  300-event regression proving bounded slices preserve all translated events.
- Added per-generation shared snippet identity and command metadata to
  `ValidatedConfig`. Match plans, pending results, and output transactions
  retain IDs, triggers, and command configs through `Arc`; form templates also
  remain shared until rendering. Regression tests assert pointer identity with
  the validated snapshot and verify command/form matching does not copy
  unused replacement text.
- Removed the manual `unsafe impl Send` for the libei event pump. The libei
  session and event converter are created, used, and destroyed on one dedicated
  owner thread; the injector exposes a bounded request/reply handle. Compile-
  time checks keep both session types `!Send`.
- Check the exact `SO_PEERCRED` structure length before using kernel-returned
  credential fields; tests exercise the expected, short, and oversized lengths.
- Add public `Matcher::try_new` and `MatcherBuildError`, making arbitrary
  trigger collections fail explicitly when their indices exceed the trie
  representation. Keep the former `new` constructor as a deprecated
  compatibility wrapper; the validated engine path uses the fallible API and
  retains its documented limit invariant.
- `cargo test -p action-broker --lib
  ipc::tests::peer_credentials_require_the_exact_kernel_structure_size
  --locked`: passed.
- `cargo test -p wayexpand-core matcher::tests --locked`: all 8 matcher tests
  passed, including the reserved-index boundary regression.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
  with the fallible matcher API and all downstream call sites.
- `RUST_TEST_THREADS=1 cargo test --workspace --locked -q`: passed; all
  executed workspace unit, integration, and documentation tests succeeded.
- Ran `wayexpand doctor --certify --json` against private headless KWin 6.6.6
  at commit `30e4557c1a96`. The clean-build report is preserved at
  [`docs/certification/evidence/kwin-6.6.6-private-30e4557c.json`](docs/certification/evidence/kwin-6.6.6-private-30e4557c.json).
  Twelve output/focus checks passed and backend latency was measured; four
  scenarios were not tested. The report correctly says `certified: false`.
  This does not satisfy physical-capture or production-certification gates.
- Hardened command queue poison recovery: it checks the bounded-length
  invariant before clearing poison and refuses queue operations when that
  invariant is broken. This state is reported as worker unavailability rather
  than queue saturation. Regression tests cover valid recovery, ordinary
  saturation, and fail-closed behavior for an intentionally over-capacity
  poisoned queue.
- `cargo test -p wayexpand-core engine::async_runtime::tests --locked`: all 3
  queue tests passed; after distinguishing saturation from unavailable-state
  failures, all 4 queue tests passed. `RUST_TEST_THREADS=1 cargo test
  -p wayexpand-core --locked -q`: 456 passed, 0 failed, 2 ignored, plus all
  core integration/documentation tests; `cargo clippy -p wayexpand-core
  --all-targets --locked -- -D warnings`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
  after the libei ownership change.
- `RUST_TEST_THREADS=1 cargo test --workspace --locked -q`: passed after the
  libei ownership change; workspace unit, integration, and documentation tests
  passed; environment/manual-session tests remained ignored.
- CI run [40990700720](https://github.com/cyberducttape/wayexpand/actions/runs/40990700720)
  completed successfully for `f8c22209`; all jobs, including the repaired
  ShellCheck gate, passed.
- Fixed the serialized evdev path so an ordinary `Key` event with no undo
  transaction does not start an empty output batch. That batch had opened the
  40 ms post-output quarantine and discarded the translated `Text` event that
  followed, preventing typed triggers from reaching the matcher.
- Added `ordinary_key_event_does_not_quarantine_its_following_text_event`,
  which feeds the physical-key/text event order through a complete `.ssl`
  trigger and verifies the output transaction.
- Recorded the change in `CHANGELOG.md` and committed it as
  `67ef40a09214a5e6d316889c61d4f5641c858a3d` (`Preserve evdev text events
  before expansions`). The commit was pushed over SSH and verified against
  `refs/heads/main`.
- Inspected the production acceptance checklist, roadmap, certification
  matrix, and certification workflow. `gh issue list --state open` returned
  no open issues.
- Created `DEVELOPMENT_PLAN.md` with ordered P0/P1/P2 milestones and objective
  acceptance evidence.
- Completed the full locked workspace test run. Fixed two workspace Clippy
  errors in the offline library checkout path by removing redundant borrows;
  the affected CLI suite passed again after that correction.

## Verification evidence

- Fresh verification on local source `5ed43432d4f8` (2026-10-30):
  `cargo test --locked -p wayexpand-core` passed (467 unit tests, 2 ignored;
  documentation configuration and snapshot-consistency tests passed),
  `cargo test --locked -p wayexpand-daemon latency::tests` passed (6 tests),
  `cargo clippy --workspace --all-targets --locked -- -D warnings` passed,
  and `cargo fmt --all -- --check` plus `git diff --check` passed. The code
  already includes borrowed matcher inputs, stack-buffer delimiter handling,
  nonblocking matcher telemetry with a dropped-sample counter, and reusable
  caller-owned immediate/deferred result buffers; corresponding matcher,
  delimiter, buffer-reuse, and telemetry contention tests are present. These
  checks do not establish zero allocations for every event or change any
  compositor certification gate.
- `cargo run --locked --release -p wayexpand-core --example
  input_path_allocations` on the same source passed its assertions: the warmed
  immediate and deferred ASCII nonmatching paths each performed 0 allocations
  across 100,000 decoded scalar events. This narrowly covers core event
  processing with retained result buffers; it excludes backend decoding,
  matching hits, and desktop output.
- The live GitHub Fuzz workflow `41038804679` is testing remote commit
  `f6c338b42f40`, not this local commit; it was still in progress when checked.
  Its result must not be attributed to the local source.

- CI for `894cb6bb` initially failed only in `Script Validation`: ShellCheck
  could not follow the `certification-processes.sh` source in
  `kwin-certification-session.sh`. Added the missing source directive;
  `shellcheck scripts/*.sh` now passes locally. The same CI run's Rust tests,
  debug/release Clippy, headless Sway E2E, unit validation, and security audit
  jobs passed.
- An earlier CI run for `27e83eac` also failed debug Clippy on two needless
  borrows in `crates/cli/src/commands/library.rs`; both were removed in
  `894cb6bb`. The current CI run confirms workspace Clippy passes.
- `cargo fmt --all -- --check`: passed.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-core --locked`:
  452 passed, 0 failed, 2 ignored; the documentation configuration tests and
  snapshot consistency test also passed.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-backend-evdev --locked`:
  21 passed, 0 failed, including bounded-slice, event-retention, and
  cross-device activity regressions.
- `cargo clippy -p wayexpand-core -p wayexpand-backend-evdev --all-targets
  --locked -- -D warnings`: passed.
- `cargo check -p wayexpand-core --benches --locked`: passed.
- `cargo check --workspace --locked`: passed after introducing shared
  validated expansion metadata.
- `cargo test -p wayexpand-daemon --locked --no-run`: passed, compiling the
  daemon tests against the shared command metadata contract.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-core -p wayexpand-daemon
  --locked`: 452 core tests passed, 2 ignored; 2 documentation configuration
  tests and 1 snapshot consistency test passed; all 205 daemon tests passed.
- `cargo test --workspace --locked --no-run`: passed after carrying shared
  result strings and form templates through the async output path.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Criterion worst-case grapheme benchmark (4,096 combining scalars):
  65.7–68.9 µs over 10 samples. This is a scenario measurement, not a
  comparison against the pre-change implementation.
- `git diff --check`: passed.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-daemon --locked`: 205 passed,
  0 failed. This includes the new event-order regression.
- `cargo clippy -p wayexpand-daemon --all-targets --locked -- -D warnings`:
  passed.
- `RUST_TEST_THREADS=1 cargo test --workspace --locked`: passed with exit code
  0. All executed unit, integration, and documentation tests passed; two
  environment-dependent compositor/systemd tests and two manual display tests
  were ignored.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand --locked` after the Clippy fix:
  128 passed, 0 failed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
  after removing needless borrows in `crates/cli/src/commands/library.rs`.
- `cargo fmt --all -- --check`: passed after the CLI cleanup.
- `scripts/check-production-certification.sh true tests/certification/compositor-matrix.json`:
  failed closed as expected because there are zero production-required routes.
  The compatibility-mode invocation passed and explicitly remained
  non-certifying.
- Live status during investigation: daemon connected, active config valid,
  19 injection samples, no recorded recent expansion failures. The daemon
  journal contains successful `.ty` insertions. This confirms observed
  transactions only; it is not compositor certification or proof of visible
  client contents.
- `wayexpand explain .ty --json`: enabled and predicted to expand.
- `wayexpand explain .test --json`: no snippet uses that trigger in the
  active configuration.
- This automated evidence does not satisfy the real-desktop certification or
  long-soak acceptance criteria below.

## Open release blockers

- No compositor/backend route is production-certified. The current matrix has
  no production-required eligible target.
- The GitHub Actions runner inventory returned zero online self-hosted
  certification runners on 2026-10-30, so the production compositor workflow
  cannot execute a real-session target in the current repository setup.
- Rechecked the release guard: `scripts/check-production-certification.sh
  true tests/certification/compositor-matrix.json` exits 1 with the expected
  no-eligible-route message; the current live Actions runner inventory is
  still empty. Compatibility CI cannot satisfy this gate.
- Physical evdev capture during long non-atomic output, focus races, partial
  insertion, password-field behavior, runtime layout changes, and suspend /
  reconnect still require attended real-session evidence.
- Historical private-session overlap reports reproduce text corruption with
  simulated capture. The input monitor and unit tests are containment evidence,
  not a replacement for physical-capture target-integrity testing.
- No 72-hour real-desktop soak or complete published-package upgrade rehearsal
  is recorded.

## Next steps

1. Add/maintain a live-session capture test that measures physical input and
   final target text during a long replacement; keep the route uncertified
   until it passes or automatic replacement is refused before erasure.
2. Establish one protocol-capable, production-eligible compositor route and
   obtain its required runner and reviewed evidence.
3. Obtain attended real-session evidence for one release-target compositor;
   automated workspace checks cannot establish physical capture and target
   text integrity.
4. Run the 72-hour desktop soak and published-package upgrade/rollback
   rehearsal after the real-session replacement gate passes.
5. Provision an authorized compositor-labelled runner and reviewed driver for
   the production certification workflow. Current code-level and private KWin
   checks cannot establish physical capture integrity, password-field
   behavior, layout switching, or long-term recovery; the live runner inventory
   is empty and the production gate correctly fails closed.

## Latest verification and changes: 2026-10-30

- Reused NFC normalization scratch storage in `ExpansionEngine` for the
  decomposed-text suffix path. Existing grapheme normalization and normalized
  to raw source-offset mapping are preserved. A regression verifies composed
  trigger matching against decomposed input, exact matched source text, and
  stable scratch capacities after warmup.
- `cargo test --locked -p wayexpand-core`: 466 passed, 0 failed, 2 ignored;
  documentation configuration tests and snapshot consistency passed.
- Targeted Criterion `unicode_worst_case_keystroke/near_miss_trigger/zalgo`
  measured 4.180 µs before and 3.895 µs after scratch reuse (15 samples, 1 s
  warmup, 2 s measurement). This is a single-host microbenchmark, not an
  end-to-end latency claim; details are in `docs/BENCHMARKS.md`.
- Investigated the intermittent daemon concurrency stress failure from GitHub
  CI run [41157338093](https://github.com/cyberducttape/wayexpand/actions/runs/41157338093):
  a reload result could be promoted after its source file had changed. Reload
  builds now retry when fleet metadata changes during construction, and the
  reactor discards stale completed snapshots and rebuilds the current source.
  Added a deterministic stale-promotion regression.
- `cargo test --locked -p wayexpand-daemon -- --test-threads=8`: 208 passed,
  0 failed. The updated daemon concurrency stress run passed 20/20 iterations,
  each running all 208 daemon tests.
- The prior exact-head CI exposed the concurrency failure, so that run is not
  recorded as passing. Its separate SSH push workflow completed successfully.
  The source fix still needs a fresh exact-commit CI result after it is
  committed and pushed.
- On clean commit `4663ff4d4bcc`, the private KWin E2E suite passed 15/15 and
  `doctor --certify --json` reported partial, non-certified status. Reports
  are preserved under `docs/evidence/`; capture was stdin-simulated and did
  not establish physical keyboard capture, password-field handling, or safe
  concurrent input. These records do not close the production certification
  gate.


## Key-event and dispatch allocation follow-up: 2026-10-30

- Replaced the daemon's `KeyChord` clone in the key-event state path with a
  borrowed `ExpansionEngine::process_key_state` operation. `KeyChord` owns a
  `String`, so this removes the clone allocation. Hotkey lookup remains a
  separate necessary operation; the previous engine call did not traverse the
  text matcher for key events.
- Added a reusable ready-result vector alongside pending results in the daemon.
  Synchronous dispatch drains it and retains its capacity; an async evdev
  transaction takes ownership of the vector when its results must outlive the
  reactor turn.
- Regression tests compare borrowed key-state behavior with the prior owned
  key event and verify ready-result capacity survives repeated dispatch.
- `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand-core
  -p wayexpand-daemon --quiet`: 467 core tests passed (2 ignored), 2 core
  documentation-config tests and the snapshot-consistency test passed, and all
  209 daemon tests passed.
- `cargo clippy --locked -p wayexpand-core -p wayexpand-daemon --all-targets
  -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check`: passed.


## Nonblocking latency instrumentation: 2026-10-30

- Confirmed the matcher recorder was already best-effort and nonblocking.
  Changed aggregate injection and segmented profile recording to `try_lock`;
  contended samples are dropped and counted independently.
- Changed aggregate, matcher, and profile status snapshots to avoid blocking
  metrics locks. Snapshot readers use cached values when a writer owns the
  window; the one-second percentile refresh remains unchanged, and profile JSON
  is serialized after releasing its window lock.
- Added regression tests that hold metric locks while recording or reading and
  verify calls return without waiting and dropped-sample counters advance.
- Exposed `injection_latency_dropped_total` and
  `injection_latency_profile_dropped_total`, updated the stable status contract,
  and bumped the daemon status schema to 15. Increased the shared bounded
  control response cap from 8 KiB to 12 KiB after the expanded status crossed
  the previous cap in saturated-profile tests.
- `RUST_TEST_THREADS=1 cargo test --locked -p wayexpand-core -p
  wayexpand-daemon -p wayexpand-gui -p wayexpand --quiet`: 467 core tests
  passed (2 ignored), 2 core config-doc tests and snapshot consistency passed,
  212 daemon tests passed, 174 GUI tests passed, and 131 CLI tests passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, Markdown-link validation (54 files, 483 local
  links), documentation-contract checks, and `git diff --check`: passed.


## Wayland health dashboard and KWin tracker lifecycle: 2026-10-31

- Promoted the session report refresh action to the top of the Wayland
  compatibility dashboard. The panel now says when diagnostics have not yet
  run, when the current report was refreshed, and when its snapshot is more
  than a minute old. This avoids presenting an old route/latency snapshot as
  current. The dashboard continues to label runtime capability reports as
  diagnostics, not app tests or production certification.
- Added accessible GUI regressions for initial no-evidence state and stale
  report messaging. The focused diagnostics GUI tests passed (4/4); GUI
  Clippy with warnings denied, formatting, and whitespace checks passed.
- Combined verification passed: `RUST_TEST_THREADS=1 cargo test --workspace
  --locked --quiet` (all workspace test binaries passed; 2 tests ignored by
  their existing annotations), `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo fmt --all -- --check`, and `git diff
  --check`.
- KWin tracker temp files now use mode 0600 independent of umask. Startup
  cleanup only removes a strictly named, private, single-link file whose
  exact script contents are recognized, whose PID is no longer active, and
  whose corresponding KWin script is confirmed unloaded. Unknown states and
  active scripts are preserved. Follow-up review bounded each cleanup to 512
  directory entries and at most four compositor checks; file verification
  opens with `O_NOFOLLOW` and reads only the expected script size. Added parser,
  cleanup-safety, and work-bound tests. The live KWin tracker health test
  passed on Plasma/KWin 6.6.6 and checked the file mode.
- After the cleanup bounds were added, the full serial workspace test suite
  passed (all test binaries passed; 2 tests ignored by existing annotations),
  workspace Clippy with warnings denied passed, Markdown validation checked
  54 files and 485 local links, documentation contracts passed, and formatting
  and diff checks passed. The corresponding hosted CI run for the earlier
  dashboard commit remained queued throughout this verification; the bounded
  cleanup follow-up still needs an exact-head hosted run.
- Production compositor certification remains open: these local tracker and
  capability checks do not validate physical keyboard capture, password-field
  behavior, or correct insertion across real target applications.
- A fresh `cargo run --quiet --locked -p wayexpand -- doctor --certify --json`
  run on clean pushed commit `48a39fb3f63b` passed 13 isolated output/focus
  checks, measured 20 injector transactions per size, and left four scenarios
  untested. The build identity matched. Capture was stdin-simulated on private
  headless KWin 6.6.6; the report remains uncertified and is preserved at
  [`docs/evidence/doctor-certify-48a39fb3.json`](docs/evidence/doctor-certify-48a39fb3.json).
  The real-session certification gate remains open.

## Dependency policy audit: 2026-10-30

- `cargo audit --deny yanked` passed with no vulnerability advisories and no
  yanked locked dependencies. Three informational unmaintained advisories
  affect the zbus 5 dependency lineage; `deny.toml` already records the
  rationale for allowing them while current desktop integration dependencies
  still require that major version.
- `cargo deny check` passed. Removed 12 obsolete duplicate-version skips, one
  unmatched Windows skip, and an unused BSL-1.0 license allowance. The check
  still reports eight duplicate-version groups from upstream/platform-specific
  dependency trees; these remain visible for review rather than being
  blanket-suppressed.
- Dependency audit findings do not change the status of the open real-session
  certification and production acceptance gates.

## Exact-head workspace verification: 2026-10-30

- On commit `01671a20c3b11d1b620d2272b9e01d3e27ef7f82`,
  `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet` passed across
  all workspace test binaries; tests marked ignored by their existing
  annotations remained ignored.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed.
- GitHub CI for this exact head is queued, as are CodeQL and Launchpad sync.
  The repository currently has no registered Actions runners (`total_count: 0`);
  all 31 CI jobs are queued without a runner assignment. This is not a hosted
  CI pass. Local checks do not replace the pending hosted build matrix or the
  open real-session certification gate.

## Focused private KWin client matrix: 2026-10-30

- Ran `scripts/kwin-certification-prepare.sh` from clean exact source commit
  `f59873ba62d2` in a private KWin 6.6.6 session, with stdin-simulated capture,
  production libei keysym fallback, one `us` layout, and a real GTK text
  client. Seven scenarios passed: rapid multiline typing, cross-window focus,
  focus change during expansion, target closure, configuration reload, daemon
  restart, and fail-closed insertion refusal.
- Preserved the machine-readable result store at
  [`docs/evidence/kwin-real-client-f59873ba62d2.txt`](docs/evidence/kwin-real-client-f59873ba62d2.txt)
  and linked it from `DEVELOPMENT_PLAN.md`. This does not exercise physical
  keyboard capture, other layouts or clients, concurrent physical input during
  paced output, password fields, or suspend/resume; it is partial evidence and
  does not close the certification gate.

## Rust 1.92 MSRV correction: 2026-10-31

- Reproduced the declared MSRV failure: the GUI's egui/eframe 0.37 dependency
  requires Rust 1.95, while the workspace and Ubuntu 26.04 source package
  contract require Rust 1.92. The existing MSRV CI job was a false green
  because plain `cargo` followed the repository's `rust-toolchain.toml` pin
  and ran Rust 1.96 instead of 1.92.
- Moved the GUI to egui/eframe 0.35 and adapted accessibility widget types,
  stable IDs, row identity, and screenshot capture to that API. Replaced emoji
  interface marks with vector-drawn icons and made the font regression test
  install the application's actual font fallbacks.
- Updated CI to print and invoke `rustc +1.92.0` / `cargo +1.92.0` explicitly,
  so the repository toolchain override cannot mask future MSRV regressions.
- Verification: Rust 1.92 GUI tests passed (175 tests); both the locked
  workspace check and full locked release workspace tests passed under Rust
  1.92; the GUI screenshot script rendered
  successfully in a private KWin session and the resulting image was visually
  inspected; Rust 1.96 Clippy passed for the GUI and all targets with warnings
  denied; formatting and `git diff --check` passed.
- An initial release-test attempt filled the 7.2 GiB `/tmp` tmpfs. No source
  files were affected; the rerun used a disk-backed target directory and
  completed successfully.
- Re-running `cargo deny check` against the downgraded GUI dependency graph
  found BSL-1.0 metadata on Windows-only `clipboard-win` and `error-code`, both
  pulled in by egui-winit's clipboard feature. Added version-pinned BSL-1.0
  exceptions for those crates rather than allowing the license globally;
  `cargo deny check` passed (with the existing duplicate-version warnings),
  and `cargo audit --deny yanked` passed with only the three already-allowed
  zbus lineage unmaintained advisories.
- The real-session physical-input certification blocker remains open; this
  toolchain compatibility fix does not change that release gate.

## Exact-head hosted CI and Launchpad validation: 2026-10-31

- GitHub CI run [41212740021](https://github.com/cyberducttape/wayexpand/actions/runs/41212740021)
  completed successfully for exact commit `584e38c6a4cb10551f89cd9b867620c123377fc9`:
  all 33 jobs passed, with no failures, cancellations, or skipped jobs.
- Coverage included locked debug and release workspace tests, debug and release
  Clippy, the explicit Rust 1.92 MSRV suite, seeded and concurrent stress tests,
  security/dependency checks, clean-source build, native aarch64 build,
  Debian/RPM artifact builds, Ubuntu 26.04 and Fedora package-install smoke
  tests, headless KWin/Sway E2E, and the three-minute daemon soak.
- Launchpad sync [41212740148](https://github.com/cyberducttape/wayexpand/actions/runs/41212740148)
  also passed for the same exact commit. At validation time the tested
  worktree was clean and `main` was synchronized with `origin/main`.
- This confirms hosted software-build and packaging checks for this commit.
  It does not establish production compositor certification, physical-input
  replacement integrity, password-field behavior, or the required 72-hour
  real-desktop soak. The repository still has zero registered certification
  runners; the release gate remains open.
- Next steps: provision an eligible real-session certification runner and
  collect/review production evidence; obtain authorization before applying the
  prepared GitHub `main` protection ruleset.

## Concurrent daemon explain requests: 2026-10-31

- Fixed a control-server race where concurrent `wayexpand explain` requests
  replaced one another in a single pending-request slot. Requests now enter a
  bounded FIFO and receive their own response; a full queue returns an
  explicit retryable busy response. The reactor drains a fixed snapshot so a
  stream of diagnostics cannot starve input processing.
- Added regressions for concurrent request/reply ownership and queue
  saturation.
- Verification: `cargo fmt --all -- --check`, both focused regressions,
  `cargo test --locked -p wayexpand-daemon` (215 passed),
  `cargo clippy --locked -p wayexpand-daemon --all-targets -- -D warnings`,
  and `git diff --check` passed.
- This fixes the observed control-plane timeout under concurrent diagnostics;
  it does not change the open real-desktop certification or production release
  gates.

## Daemon control-socket access policy: 2026-10-31

- Validate Linux `SO_PEERCRED` on every connection, require the connecting UID
  to equal the daemon UID, and reject malformed credential lengths.
- Add the root-managed organization-policy setting
  `disable_control_socket_mutations`. When enabled, it refuses insert,
  insert-target, reload, pause, resume, stop, and usage-stat clearing, while
  retaining status, focus, and explain. This access-control setting remains
  enforced even when snippet policy is in audit mode.
- Add regressions proving a client cannot replay a visible, current focus
  generation/token to insert in restricted mode, cannot change lifecycle
  flags, and cannot clear stats; add coverage for every mutation verb and
  peer-UID checks.
- Limitation: peer credentials distinguish Unix UIDs, not applications under
  one UID. The focus token remains a public freshness value. Managed read-only
  mode mitigates daemon-socket invocation; a trusted per-application consent
  mechanism remains open, and same-UID processes retain other user-session
  privileges.
- The isolated installation smoke test passed. Its release-artifact substep
  stopped because adding the daemon's direct `libc` dependency required a
  `Cargo.lock` update while the script invokes Cargo with `--locked`; the lock
  entry was updated and the complete release checks were rerun successfully.
- Verification on commit `66fb852414ad86d56a86c2a021242ee09f4f78a0`:
  `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet` passed (all
  workspace suites; 2 ignored core tests), as did the same full test command
  under Rust 1.92. `cargo clippy --workspace --all-targets --locked -- -D
  warnings` and `cargo +1.92.0 clippy --workspace --all-targets --locked --
  -D warnings` passed. `scripts/test-install-user.sh`,
  `scripts/test-install-release.sh`, and `scripts/test-release.sh` passed; the
  latter verified reproducible release archives, checksums, staged install,
  CLI smoke behavior, and source packaging. `cargo fmt --all` and
  `git diff --check` passed.
- Commit `66fb8524` was pushed to `main` over SSH with author and committer
  `Stephan Loesevitz <stephan.loesevitz@gmail.com>`. Its Launchpad sync, fuzz,
  and push workflows were still in progress and full CI was queued at the time
  this entry was written; hosted checks are not yet claimed as passing.

## Best-effort sensitive-memory cleanup: 2026-10-31

- Added explicit zeroization for the engine's live typed-character deque,
  reusable Unicode matching scratch, evicted/backspaced characters, invalidated
  undo copies, deferred trigger reservations, and command-cache values. Undo
  text is no longer retained when no undo chord is configured.
- Wipe owned text events after synchronous processing and prefetched clipboard
  values after consumption, replacement, expiration, or shutdown. Command cache
  expiration now clears entries on daemon reactor turns and uses a bounded
  deadline index so expired command output is not retained indefinitely.
- Added regressions that inspect retained backing allocations after buffer,
  scratch, undo, clipboard, and shutdown cleanup, plus cache expiry coverage.
- Documented scope and limitations in `SECURITY.md`: immutable configuration
  and returned results, in-flight data, stack/register copies, allocator/OS
  copies, and GUI processes outside the shipped systemd units are not covered
  by these targeted wipes. All four shipped units already set `LimitCORE=0`.
- Verification: `RUST_TEST_THREADS=1 cargo test --workspace --locked --quiet`
  passed on the current toolchain and Rust 1.92 (core: 475 passed, 2 ignored;
  daemon: 219 passed; UI: 175 passed; all other workspace suites passed).
  Current-toolchain and Rust 1.92 workspace Clippy with `-D warnings`,
  `cargo deny check`, `cargo fmt --all -- --check`, and `git diff --check`
  passed. `cargo deny` reports pre-existing duplicate-version warnings; its
  advisories, bans, licenses, and sources checks pass.
- This reduces residual copies in selected owned buffers; it is not a guarantee
  of complete process-memory erasure or protection from a privileged memory
  dump. Real-desktop security and certification gates remain open.

## Diagnostic privacy hardening: 2026-10-31

- Focus-change insert refusals now log a fixed `focus_changed` reason without
  expected or actual window tokens. A scoped tracing-subscriber regression
  captures the emitted record and checks it contains neither token.
- `CommandError::NonZeroExit` now contains only the exit status. Direct command
  stderr is drained in bounded reads and discarded, preventing child-provided
  secrets from entering `Display`, `Debug`, UI error strings, or daemon failure
  traces. A process regression prints a recognizable synthetic secret to
  stderr, exits unsuccessfully, and verifies the resulting error is
  content-free.
- Verification: both focused regressions pass with `--locked`; full locked
  workspace tests pass on the current toolchain and Rust 1.92 (core: 476
  passed, 2 ignored; daemon: 220 passed; UI: 175 passed; all other workspace
  suites passed). Workspace Clippy with `-D warnings` passes on both toolchains,
  as do `cargo fmt --all -- --check` and `git diff --check`.

## Restricted focus diagnostics and exact-head CI: 2026-10-31

- In managed read-only socket mode, `focus` continues to report the active
  generation and exact-identity availability but returns an empty focus token.
  Since that mode rejects all mutation requests, exposing a replayable target
  token provided no supported operation. Unrestricted picker behavior remains
  unchanged. The same-UID authorization limitation remains open.
- Updated the control-socket regression and changelog/plan to record this
  narrower mitigation without treating a focus token as authorization.
- Hosted CI for commit `59aaa7cee9fab902a7bf04fd0d81e7b41a43b1b9` completed
  successfully: all 33 jobs passed, including release tests, lint, package
  artifacts, integration tests, clean source build, and headless KWin/Sway
  output checks.
- The focused restricted-mode regression passed. All 219 daemon tests passed
  serially, daemon Clippy with `-D warnings` passed, and `cargo fmt --all --
  --check` plus `git diff --check` passed. An initial parallel daemon test run
  exposed a pre-existing race in the two form-reason tests: they asserted
  against the process-wide bounded trace ring while unrelated tests could
  evict their entries. Those assertions now test the pure content-free reason
  mapping directly, eliminating cross-test interference.
- Hosted CI run `41226725007` for commit
  `9d79a4c04b833238b99f96849604224df6dc70d9` completed successfully: all 33
  jobs passed, including package install smoke tests on Ubuntu 26.04 and Fedora.
