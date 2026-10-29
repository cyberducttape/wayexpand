# Production acceptance checklist

This checklist is release evidence, not a claim that every item is currently
complete. A checked item must have a code path, automated test, or preserved
artifact linked in the release notes. Desktop-session items require real
session evidence; unit tests alone do not satisfy them.

## Security

- [x] Document and enforce an acceptable backend policy. Recommended setup
      excludes evdev; organization policy can restrict allowed backends.
      See [BACKENDS](BACKENDS.md), [ORGANIZATION_POLICY](ORGANIZATION_POLICY.md),
      and `wayexpand setup`.
- [x] Require explicit acknowledgement for evdev's missing sensitive-field
      signal. See [BACKENDS_SENSITIVE_FIELDS](BACKENDS_SENSITIVE_FIELDS.md).
- [x] Verify broker socket ownership, permissions, trusted ancestors, socket
      identity, bounded frames, peer identity, and action policy in automated
      tests. See `crates/action-broker/src/ipc.rs` and `path_security.rs`.
- [ ] Demonstrate safe behavior on unexpected focus changes in real desktop
      sessions.

## Correctness

- [ ] Pass a compositor-specific integration matrix. See
      [CERTIFICATION_MATRIX](CERTIFICATION_MATRIX.md).
- [ ] Record zero unintended deletions, duplicate insertions, or wrong-window
      injections across the complete supported-client regression corpus.
- [ ] Pass partial-insertion fault-injection tests against every approved
      output backend.
- [ ] Pass rapid-typing, Unicode, and layout-change tests for every approved
      backend/layout combination.
- [ ] Type a second trigger during a long, non-atomic replacement on each
      candidate route using certification scenario
      `second-trigger-during-nonatomic-replacement`; verify the target text is
      not corrupted and no stale trigger is expanded after the first
      transaction. A sequential event test does not satisfy this gate.
      The [negative KWin evidence](evidence/kwin-overlap-reproduction-2026-10-28.json)
      records target corruption and stale-trigger expansion on an older dirty
      revision with stdin-simulated capture. Current serialized evdev output
      clears matcher context and discards queued events through a 40 ms quiet
      window; a daemon regression covers this filtering behavior. That change
      cannot stop physical input reaching the application during injection,
      and the recorded compositor scenario has not been rerun against the
      current source with physical capture. Keep this gate open until the
      current route passes the live target-integrity scenario or is refused
      for automatic replacement.
- [ ] Verify command completion never inserts into an unintended context in a
      real focus-switch test.
- [ ] Verify fail-closed behavior in password and credential fields for each
      route that claims sensitive-field protection.

## Operations

- [ ] Pass crash/restart and suspend/resume tests in real user sessions.
- [x] Verify atomic upgrade and rollback under injected installer failures,
      including SIGTERM after version switching. See
      `scripts/test-install-user.sh` and `scripts/test-install-release.sh`.
- [x] Provide machine-readable diagnostic status and bounded failure counters.
      See the status schema and `docs/INTEGRATION_TESTING.md`.
- [x] Provide signed release provenance and reproducible build instructions.
      See [RELEASING](RELEASING.md) and [DEVELOPMENT](DEVELOPMENT.md).

## Performance

- [ ] Publish p50/p95/p99 insertion latency per approved backend.
- [ ] Complete a 72-hour real-desktop soak on each approved route, including
      suspend/resume, compositor reconnect, daemon restart, and configuration
      reload.
- [ ] Demonstrate stable memory, thread count, and file descriptors during
      that soak. The isolated soak harness now enforces peak RSS, descriptor,
      and thread-count growth limits; it does not replace this desktop test.
- [x] Demonstrate bounded behavior and recovery under event-queue saturation.
      Evdev and input-method event queues reset stale input and accept fresh
      input after overflow; the serialized output queue rejects excess work
      and accepts new transactions after draining. Regression tests are
      `pending_event_overflow_resets_the_queue_and_accepts_fresh_input`,
      `protocol_event_queue_is_bounded_and_fails_closed`, and
      `full_transaction_queue_rejects_excess_work_and_recovers`.

The currently open items are intentional production gates. In particular,
evdev must not be treated as approved merely because its unit starts, and
desktop-independent soak results do not substitute for compositor evidence.
