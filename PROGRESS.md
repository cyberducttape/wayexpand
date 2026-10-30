# Production Readiness Progress

Updated: 2026-10-29

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
- Committed locally as `045dfdbb` (`Avoid owned strings for scalar input
  events`). The working tree is clean. The active user service was not
  deployed/restarted, and this commit has not been pushed.
- Updated the core's event-by-event Unicode trigger helper to emit
  `InputEvent::TextChar`, so canonical normalization and grapheme deletion
  cases exercise the same event shape as real scalar key input. Added a
  focused regression for decomposed-accent matching and backspace after
  scalar-by-scalar input. Reverification passed: workspace Clippy,
  `RUST_TEST_THREADS=1 cargo test --workspace --locked -q` (465 core tests
  passed, 2 ignored; 206 daemon; 173 GUI; 128 CLI; all other suites passed),
  formatting, and `git diff --check`.

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
  production certification because no matrix route is eligible. GitHub Actions
  for `60315df9` was cancelled when the evidence-only commit was pushed; no job
  failure was reported. CI run `41003908901` for evidence commit `5b3eefe7`
  completed successfully. CI run `41006119736` also completed successfully on
  the earlier commit `2614bc53`, including the Rust test/lint, Debian/RPM
  artifact, and Ubuntu 26.04/Fedora package-install smoke jobs. The local
  commits after `2614bc53`, including `045dfdbb` and `4b4cd9c3`, have passed
  local verification but have not been submitted to CI.
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
5. Add targeted lifecycle tests for a libei owner thread that exits during
   startup, a failed operation reply, and bounded shutdown while the EIS
   session is unresponsive; current tests enforce thread affinity but do not
   emulate a compositor session.
