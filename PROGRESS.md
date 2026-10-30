# Production Readiness Progress

Updated: 2026-10-29

## Completed in this work period

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
