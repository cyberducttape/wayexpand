# Production Readiness Progress

Updated: 2026-10-29

## Completed in this work period

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

## Verification evidence

- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- `RUST_TEST_THREADS=1 cargo test -p wayexpand-daemon --locked`: 205 passed,
  0 failed. This includes the new event-order regression.
- `cargo clippy -p wayexpand-daemon --all-targets --locked -- -D warnings`:
  passed.
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
- The full workspace test suite was not completed in the earlier interrupted
  run. No claim is made that all workspace suites pass in this work period.

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
3. Run the full locked workspace test and Clippy lanes, then distribution
   upgrade tests after the desktop release gate is unblocked.
