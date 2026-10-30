# Production Readiness Progress

Updated: 2026-10-30

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
