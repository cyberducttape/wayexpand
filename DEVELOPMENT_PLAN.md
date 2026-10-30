# Production Readiness Development Plan

This plan uses the repository's production acceptance checklist, compositor
certification matrix, CI workflows, and open GitHub issues as its inputs. A
milestone is complete only when its acceptance evidence is recorded in
`PROGRESS.md`; synthetic tests do not satisfy real-desktop criteria.

## Release gates, in priority order

### P0 — Prove replacement integrity on a production route

**Status:** Open. No route is production-certified.

Acceptance criteria:

- Select at least one compositor/backend route that can provide the required
  capture, insertion, focus, and sensitive-field guarantees. Do not mark a
  route production-required while its protocol or runner is unavailable.
- Run the required certification scenarios from
  `tests/certification/compositor-matrix.json` against physical capture and
  actual target clients, including a second trigger during a long replacement,
  modifier input during replacement, focus changes, client closure, partial
  insertion, and recovery.
- Record exact source revision, binary identity, compositor and kernel
  versions, backend, application/toolkit versions, layouts, dates, and raw
  machine-readable results.
- Demonstrate zero unintended deletion, duplicate insertion, or wrong-window
  insertion across the supported client/layout corpus. Any unsupported
  scenario must be refused before deletion, not silently counted as a pass.
- Have the production certification workflow accept the reviewed evidence;
  a private headless session, unit test, or protocol probe alone is not enough.

### P0 — Fail closed when replacement cannot be made safe

**Status:** Partially implemented; real-session acceptance remains open.

Acceptance criteria:

- A route with neither atomic replacement nor a reliable target-text check
  refuses automatic replacement unless the user or administrator explicitly
  enables its documented compatibility mode.
- A focus change, partial insertion, timeout, or lost acknowledgement cannot
  cause blind replay or rollback against changed text.
- Regression tests cover each transaction terminal state and the physical
  input interruption signal; real-session tests verify the client document
  after interruption.

### P1 — Complete compositor and input-context coverage

**Status:** Open.

Acceptance criteria:

- Certify supported focus tracking and app filters per compositor; missing
  application identity continues to fail closed.
- Verify password-field protection for every route that claims it. Raw evdev
  must remain explicitly unprotected and acknowledged.
- Document active IME/preedit behavior and certify supported committed-text,
  dead-key, Compose, and layout-switch workflows. Unsupported active preedit
  remains visible in the support matrix.

### P1 — Demonstrate lifecycle reliability on a real desktop

**Status:** Automated portions exist; real-session soak remains open.

Acceptance criteria:

- Complete a 72-hour soak per production route with typing, suspend/resume,
  compositor/portal reconnect, daemon restart, configuration reload, and device
  reconnect.
- Preserve machine-readable results showing bounded memory, descriptors,
  threads, queues, restart counts, and latency percentiles.
- Test upgrade and rollback on supported distributions, including upgrade from
  a previously published package when one exists.

### P1 — Establish release and distribution evidence

**Status:** Open; release publication is correctly gated by certification.

Partial evidence: hermetic per-user and release-archive installer tests cover
staged installation, validation preflight, rollback after filesystem, signal,
service restart, and service enable failures, immutable payload checks, and
custom XDG roots. The local x86_64 release archive smoke test also builds the
binary archive, verifies its checksum and required payload, and exercises the
packaged CLI. Binary archive assembly is byte-identical across differing
caller umasks on this host. These do not validate arm64 runtime behavior,
independent compiler reproducibility, package-manager install/upgrade, actual
service behavior, or published artifact signing/provenance on supported
release hosts.

Acceptance criteria:

- Build locked, reproducible artifacts for supported architectures and package
  formats; validate signatures and provenance.
- Pass package install, upgrade, rollback, and uninstall smoke tests on each
  supported distribution.
- Keep package availability, backend certification, and support status
  separate in release metadata. Do not publish a production release until P0
  gates pass.

### P2 — Close measured performance and internationalization gaps

**Status:** Open.

Partial evidence: a release-mode allocation probe in
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) measures zero allocations across
100,000 warmed, nonmatching ASCII `TextChar` events in the deferred core event
path. It excludes XKB/backend decoding, queueing, output, and match-result
construction, so it does not satisfy the end-to-end latency or large-library
profiling gates.

Acceptance criteria:

- Publish p50/p95/p99 event-to-insertion latency for each approved route and
  representative replacement size, separating matcher and backend timing.
- Profile sustained input, 10,000-snippet GUI search/edit/save, template
  rendering, and concurrent command workloads before optimizing.
- Add composition-aware Fcitx/IBus support only with a protocol design and
  corresponding real-client tests; do not infer preedit state from raw keys.

## Current highest-priority executable work

The highest-priority remaining acceptance gate requires physical keyboard
input, actual compositor focus changes, and target-client document inspection.
Repository-local work can improve drivers and invariants, but cannot produce
that evidence by itself. Until that session evidence is collected, continue
with P0 fail-closed behavior and automated regression coverage without
promoting any route to production-certified.
