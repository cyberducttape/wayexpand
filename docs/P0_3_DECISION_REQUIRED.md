# P0-3 Decision: Evdev Is Best-Effort, Not Atomic

**Date:** 2026-09-19  
**Status:** Decision recorded — document the limitation; do not implement a
non-exclusive queueing pseudo-fix
**Impact:** evdev cannot support a guaranteed-correct expansion claim

## The problem

Evdev observes raw keyboard events while the focused application receives the
same physical events independently:

```text
physical keyboard
      ├──> application
      └──> WayExpand observes events
                    ↓
              trigger detected
                    ↓
              inject backspaces/text
```

When a user types quickly, the terminating space, Enter, or next character may
already be in the application before WayExpand decides to erase the trigger.
The `reinsert_after` handling and bounded quiet period reduce the common race,
but they cannot make delivery atomic.

Possible outcomes include an unexpanded trigger, duplicated punctuation, or an
overlapping replacement. This is a property of non-exclusive observation, not
just a missing queue in the matcher.

## Decision

Keep evdev as an explicitly enabled, best-effort compatibility backend.

- Document that fast typing and high scheduling/input latency can expose the
  replacement race.
- Do not describe evdev as atomic, lossless, or production-guaranteed.
- Keep the bounded quiet-period and held-key handling already present; these
  are useful mitigations, not correctness proofs.
- Do not add a queue in the current non-exclusive path and claim that it fixes
  the race.
- Prefer a capture/injection protocol with interception semantics when a user
  needs a correctness guarantee. Its own limitations must still be tested.

The automatic resolver already avoids enabling evdev without explicit user
acknowledgment. This decision is about the behavior after that acknowledgment,
not about silently removing the compatibility backend.

## Why a queue is not enough

WayExpand cannot hold an event before the application receives it if evdev is
non-exclusive. A queue between the evdev reader and matcher only delays
WayExpand's observation; it does not delay the application's copy of the
physical event. It can improve internal batching or throughput, but it cannot
provide atomic erase-and-replace semantics.

## Future exclusive-proxy project

True synchronization would require a materially different architecture:

```text
keyboard
    ↓
EVIOCGRAB / exclusive WayExpand proxy
    ↓
WayExpand decides what to emit
    ↓
virtual keyboard or another injector
    ↓
application
```

That is not a small evdev enhancement. Before considering it, a separate
design and threat-model review must cover:

- crash recovery and guaranteed emergency release of grabbed keyboards;
- virtual-terminal switching, suspend/resume, and compositor restart;
- multiple keyboards, hotplug, device identity, and seat isolation;
- injection loops and distinguishing WayExpand-generated events;
- reconnects, portal/injector failure, latency, and accessibility behavior;
- users being locked out of input during partial startup or failure;
- systemd supervision and a bounded recovery path that does not require a
  working keyboard.

Until those questions have tested answers on supported distributions and
desktop sessions, `EVIOCGRAB` must not be introduced as a casual correctness
patch.

## User-facing guidance

Use evdev when broad keyboard fidelity and compositor compatibility matter and
the deployment accepts both its raw-input permission model and best-effort
replacement timing. For sensitive fields, use a source that can receive field
semantics. For strict replacement correctness, use a protocol path with
interception semantics and verify it on the target compositor.

The support matrix, `wayexpand doctor`, and backend explanation must continue
to expose these distinctions rather than collapsing “can read events” into a
guarantee about replacement behavior.
