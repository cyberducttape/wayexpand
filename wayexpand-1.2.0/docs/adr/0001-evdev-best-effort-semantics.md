# ADR 0001: Evdev Uses Best-Effort Expansion Semantics

- Status: Accepted
- Date: 2026-09-19
- Decision owners: WayExpand maintainers

## Context

Evdev observes raw keyboard events while the focused application receives the
same physical events independently. WayExpand can therefore detect a trigger
only after the application may already have received its terminating space,
Enter, or following character. It cannot make deletion and reinsertion an
atomic transaction without exclusive capture.

Possible results include an unexpanded trigger, duplicated punctuation, or an
overlapping replacement during rapid input. This is an inherent property of
non-exclusive observation, not a queueing defect.

## Decision

Keep evdev as an explicitly enabled, best-effort compatibility backend. Its
documentation and diagnostics must not claim atomic expansion or the same
sensitive-field guarantees as input-method-v2. The bounded quiet period,
terminator handling, and transaction safeguards remain useful mitigations but
do not change the semantic guarantee.

## Consequences

- evdev remains available for compositors and sessions without a suitable
  exclusive input path.
- Users and administrators must opt in to its broader raw-input access and
  understand that rapid replacements can race application delivery.
- Future work may improve timing and device handling, but must not represent
  those improvements as atomicity without an exclusive capture mechanism.

## References

- [Backend behavior](../BACKENDS.md)
- [Support matrix](../SUPPORT_MATRIX.md)
- [Security and threat model](../../SECURITY.md)
- [Historical decision record](../archive/2026-09/P0_3_DECISION_REQUIRED.md)
