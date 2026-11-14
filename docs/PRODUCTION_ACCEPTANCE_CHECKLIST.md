# Production acceptance checklist

This checklist is release evidence, not a claim that every item is currently
complete. A checked item must have a code path, automated test, or preserved
artifact linked in the release notes. Desktop-session items require real
session evidence; unit tests alone do not satisfy them.

## Security

- [x] Protect the default branch with required successful CI checks, pull
      requests, resolved review conversations, and force-push/deletion
      restrictions. Verify a failing required check blocks merge. See
      [REPOSITORY_GOVERNANCE](REPOSITORY_GOVERNANCE.md). Active since
      2026-10-31; probe PR #37 with a failing required check was `BLOCKED`.
- [x] Document and enforce an acceptable backend policy. Recommended setup
      excludes evdev; organization policy can restrict allowed backends.
      See [BACKENDS](BACKENDS.md), [ORGANIZATION_POLICY](ORGANIZATION_POLICY.md),
      and `wayexpand setup`.
- [x] Require explicit acknowledgement for evdev's missing sensitive-field
      signal. See [BACKENDS_SENSITIVE_FIELDS](BACKENDS_SENSITIVE_FIELDS.md).
      The local consent marker remains per-machine, and the GUI's persistent
      route indicator uses live daemon status to distinguish verified,
      unavailable, and unknown sensitive-field signals.
- [x] Verify broker socket ownership, permissions, trusted ancestors, socket
      identity, bounded frames, peer identity, and action policy in automated
      tests. See `crates/action-broker/src/ipc.rs` and `path_security.rs`.
- [ ] Demonstrate safe behavior on unexpected focus changes in real desktop
      sessions.

## Correctness

- [x] Keep automatic replacement unavailable by default on routes without an
      atomic replacement contract. A verified-surrounding-text check alone
      does not prevent concurrent input from interleaving between erase and insertion.
      The trigger remains intact when the daemon refuses, and Quick Picker
      offers clipboard/manual paste unless the target identity is exact and
      replacement is atomic. A non-atomic route can be promoted only after it
      passes the real-session target-integrity gate below. The explicit legacy
      compatibility setting is an experimental override for typed triggers;
      it never makes Quick Picker use non-atomic output.
- [x] Keep paced keysym output bounded by a short default and preview route
      strategy. The planner recommends atomic verified replacement first,
      otherwise native text commit only under explicit compatibility consent,
      paced keystrokes only within the configured `keystroke_max_chars` limit
      (8 characters, about 100 ms, by default), and clipboard/manual paste
      when the active route cannot meet those conditions. Raising the limit is
      explicit consent to slower output. The throughput estimate is not
      certification evidence.
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
      The daemon now attempts to cancel an in-flight non-atomic transaction
      when evdev reports new physical input, polling every 2 ms while paced
      output is active, records an uncertain outcome, and quarantines the input
      instead of replaying or continuing queued output. A delayed-event unit
      regression covers the reactor polling path, but its input source is
      simulated. The monitor treats every raw key event as activity, including
      modifiers and lock keys ignored by matcher translation.
      The evdev source also watches `/dev/input` topology with inotify. A
      device-node create, remove, move, or attribute change during pending
      non-atomic output triggers an immediate rescan, and output is
      interrupted only when the rescan adds a keyboard, because only a new
      keyboard can deliver keys the monitor has not seen. Non-keyboard nodes
      (audio jacks, HDMI audio, mice), removals, and permission updates no
      longer cut a paced replacement off partway; a keyboard node that is not
      readable yet is added, and interrupts, on its following permission
      (attribute) event. A change seen while a device batch or lost slot is
      still pending interrupts without rescanning.
      Watch invalidation, unmount, read failure, and excessive event backlog
      retire the monitor instead of leaving it silently inactive.
      Unit tests verify that a non-keyboard node does not interrupt and that
      an added keyboard does, but do not verify a real keyboard hotplug
      during output.
      A separate `modifier-during-nonatomic-replacement` certification case
      now records this requirement; the headless KWin driver marks it
      unverified because it does not exercise physical capture.
      The `keyboard-hotplug-during-nonatomic-replacement` case separately
      requires reconnecting a physical keyboard while paced output is
      active. The headless driver marks it unverified because it has no
      physical input devices; monitor unit tests do not satisfy this gate.
      This is containment only: any already delivered physical keys or
      synthetic characters may leave partial text, so the target-integrity
      gate still requires a passing real-session test or refusing automatic
      replacement on that route.
      Any unknown or partial output outcome now stops the batch and quarantines
      queued evdev input even if the interruption monitor saw no new key. A
      separate `output_partial_total` counter and GUI alert call out failures
      after insertion began; uncertain output is never replayed. These changes
      reduce stale-trigger risk but do not recover already delivered text.
      The same failure was reproduced against clean commit `afdcd03633df` in
      [this report](evidence/kwin-overlap-reproduction-afdcd0363.json): 124 of
      128 replacement characters remained, the concurrent trigger was
      interleaved, and the stale second trigger expanded. Capture was simulated
      over stdin, so this does not test the evdev interruption monitor.
      The latest clean-source reproduction on commit `6ddbfa854baa` is
      [preserved here](evidence/kwin-overlap-reproduction-6ddbfa854baa.json).
      It reproduces corruption: only 124 of 128 replacement characters
      remained, the second trigger was interleaved, and stale input expanded.
      The daemon's uncertain-output counter remained zero because capture was
      simulated over stdin while the concurrent trigger was typed through an
      independent EIS session. This does not test the physical-evdev
      interruption path. The current-source reproduction against clean commit
      `0136588a3eda` is
      [preserved here](evidence/kwin-overlap-reproduction-0136588a.json): again,
      124 of 128 replacement characters remained and the stale second trigger
      expanded. The private KWin input-method-v2 probe on the same commit
      [could not find a manager/seat](evidence/kwin-input-method-unavailable-0136588a.json),
      so its four insertion, password-field, and form scenarios were
      unverified. This does not establish that input-method-v2 is unavailable
      in other KWin configurations. The prior
      [ad8f169c4402 reproduction](evidence/kwin-overlap-reproduction-ad8f169c4402.json)
      also reproduced corruption: 124 of 128 characters remained. Earlier
      reports from
      [6d18b08c1f06](evidence/kwin-overlap-reproduction-6d18b08c1f06.json)
      and [3aac97291](evidence/kwin-overlap-reproduction-3aac97291.json)
      showed the same failure. The current serialized evdev output clears matcher
      context and discards queued events through a 40 ms quiet window; the new
      input monitor attempts to stop further output at a backend checkpoint.
      Neither mechanism can stop physical input already reaching the
      application or undo partial text. The older
      [2026-10-29 report](evidence/kwin-overlap-reproduction-0803d915.json)
      and [2026-10-28 report](evidence/kwin-overlap-reproduction-2026-10-28.json)
      also record failures.
      A rerun on 2026-11-11 against commit `9da90fb36d45` (dirty worktree)
      failed. It observed only 26 acknowledged insertion units
      out of 128, with 5 erased units, interleaved text, and
      `output_partial_total=1`. The machine-readable record is
      [here](evidence/kwin-overlap-reproduction-9da90fb3.json). Capture was
      stdin-simulated, and the scenario explicitly raised the keysym limit
      from its default of 8 to 250 characters to exercise a long replacement.
      This confirms the libei output integrity failure under the opt-in long
      output setting but does not certify or falsify physical-evdev interruption
      handling.
      Keep this gate open until the current route passes the live target-integrity
      scenario with physical capture or automatic replacement is refused on
      that route. The current-source private KWin smoke report
      [9311e1e5be3f](evidence/doctor-certify-9311e1e5be3f.json) marks this
      scenario `not-tested`; it does not close the gate.
- [ ] Verify command completion never inserts into an unintended context in a
      real focus-switch test.
- [ ] Verify fail-closed behavior in password and credential fields for each
      route that claims sensitive-field protection.

## Operations

- [x] Publish the initial dated, versioned desktop certification record for
      KDE, GNOME, Sway, and Hyprland. The current release-line snapshot is
      [CERTIFICATION_MATRIX](CERTIFICATION_MATRIX.md); all four targets remain
      uncertified.
- [ ] Complete and publish physical-session certification for each target.
      Record exact compositor and WayExpand versions, route, physical keyboard,
      focused clients, screen lock/unlock, layout changes, suspend/resume,
      portal revocation, and a 72-hour session soak. Link reviewed
      machine-readable evidence for every pass and retain failures and untested
      cases. CI and headless tests do not satisfy these desktop-session checks.
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
