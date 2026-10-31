# Production Readiness Development Plan

This plan uses the repository's production acceptance checklist, compositor
certification matrix, CI workflows, and open GitHub issues as its inputs. A
milestone is complete only when its acceptance evidence is recorded in
`PROGRESS.md`; synthetic tests do not satisfy real-desktop criteria.

## Release gates, in priority order

### P1 — Require broker-mediated actions in managed profiles

**Status:** Implemented with regression coverage; managed deployment adoption
and review of each broker action remain operational requirements.

The root-managed organization policy now supports
`safe_mode = true` with `require_broker_actions = true`. Direct executable
snippets and hotkeys fail configuration validation and are checked again at
runtime, including direct hotkey execution and GUI command preview. Named
Action Broker actions remain available subject to the broker's own allowlist
and service policy; both expansion and hotkey actions use the broker protocol.
Hotkey requests disable output capture because their output is discarded.
Espanso and pack imports continue to strip external
commands; documentation now directs administrators to review and define
broker actions rather than restoring imported executables directly.

Acceptance criteria:

- Reject direct executable expansions and hotkeys when both policy flags are
  enabled, while accepting named broker actions.
- Prove a direct executable cannot run if runtime policy is applied after the
  library was constructed, and refuse it through both queued and direct
  hotkey APIs.
- Block direct GUI previews under this policy while preserving named broker
  action eligibility.
- Keep the policy audit-only when `safe_mode = false`, and publish it through
  the CLI schema and doctor diagnostics.
- Document import behavior and the limits of broker isolation; each action
  still requires administrator review and broker configuration.

### P0 — Prove replacement integrity on a production route

**Status:** Open. No route is production-certified.

The latest exact-head smoke report is
[`docs/evidence/doctor-certify-e54af849.json`](docs/evidence/doctor-certify-e54af849.json),
run on clean commit `e54af8492ef4` in private KWin 6.6.6. Output, focus,
reload/restart, and safe-refusal checks passed; the keysym fallback measured
20 transactions per size (p95 12.715 ms small, 780.415 ms medium, 3.126 s
large). Capture was stdin-simulated. Physical capture, overlap and hotplug
during replacement, Unicode insertion, password fields, and suspend/resume
remain untested. The report is partial compatibility evidence, not
certification.

An earlier `doctor --certify` report ran from clean commit
`48a39fb3f63b` on private headless KWin 6.6.6. The preserved report at
[`docs/evidence/doctor-certify-48a39fb3.json`](docs/evidence/doctor-certify-48a39fb3.json)
records 13 passing checks, four untested scenarios, and 20 injector latency
samples per replacement-size bucket. Capture was simulated over stdin; the
report is explicitly partial and does not satisfy physical capture, overlap,
password-field, suspend/resume, or production certification criteria.

The latest focused current-source KWin real-client run is preserved in
[`docs/evidence/kwin-real-client-fc063a4054ab.summary.json`](docs/evidence/kwin-real-client-fc063a4054ab.summary.json)
and its raw store. All 35 selected cells passed across five target clients
for multiline typing, cross-window focus, pending-output focus changes, target
closure, configuration reload, daemon restart, and pre-delete refusal of
unsupported Unicode output. The run used private KWin 6.6.6, the `us` layout,
stdin-simulated capture, and libei keysym output. It is compatibility evidence
only; physical capture and overlapping-input integrity remain unverified, and
the previously reproduced non-atomic overlap corruption remains a release
blocker.

Additional focused output-path evidence was collected on exact source commit
`f59873ba62d2` in a private KWin 6.6.6 session. Seven scenarios passed against
a real GTK text client: rapid multiline input, cross-window focus, focus
change, target closure, configuration reload, daemon restart, and refusal
before insertion. The result store is preserved at
[`docs/evidence/kwin-real-client-f59873ba62d2.txt`](docs/evidence/kwin-real-client-f59873ba62d2.txt).
Capture was stdin-simulated and only the `us` layout and GTK client were used;
this adds output/focus regression evidence but does not satisfy the real
capture, layout, password-field, overlap, or production certification gates.

Earlier partial evidence: `doctor --certify` ran from clean commit
`c94a4f634bfb` in a private KWin 6.6.6 session. The preserved report at
[`docs/evidence/doctor-certify-c94a4f634bfb.json`](docs/evidence/doctor-certify-c94a4f634bfb.json)
records passing replacement, focus, reload, restart, and refusal checks, plus
20 injector latency samples for each size bucket. It explicitly leaves
physical capture, overlap input, password fields, suspend/resume, and long soak
untested, and reports `certified: false`. This is current-source output-path
evidence only; the release gate remains open.

The latest `doctor --certify` run at commit `494aac93b15d` is preserved in
[`docs/evidence/doctor-certify-494aac93.json`](docs/evidence/doctor-certify-494aac93.json).
It passed 13 checks, measured 20 injector samples in each size bucket, and
left four scenarios untested. It still uses private headless KWin and
stdin-simulated capture, so it does not close this gate; its p99 for a large
replacement was 3.16 seconds on the keysym fallback.

On the same source commit, the opt-in private KWin overlap scenario reproduced
corruption: 124 of 128 replacement characters remained and the stale second
trigger expanded ([failure report](docs/evidence/kwin-overlap-reproduction-0136588a.json)).
Capture was simulated over stdin, so this confirms the non-atomic output race
but does not test the physical-input monitor. A separate private KWin
input-method-v2 attempt could not find a manager/seat; all four route-specific
scenarios remained unverified ([availability report](docs/evidence/kwin-input-method-unavailable-0136588a.json)).
The supported production route still needs an actual compositor setup that
offers the required protocol and the required physical-capture evidence.

The active KDE desktop was also checked read-only on 2026-10-30. Its installed
daemon is an older evdev + libei keysym fallback route with best-effort,
non-atomic replacement and no password-field signal; the current doctor exits
unhealthy. It is useful live status evidence, but does not satisfy certification
or justify changing the installed service. Details are in `PROGRESS.md`.

The GitHub repository runner API returned `total_count: 0` on 2026-10-31 UTC.
The certification workflow requires an online self-hosted runner labeled
`wayexpand-certification` and the compositor ID. Until at least one eligible
runner is registered and online, real-session certification cannot be
collected by the repository workflow. This is an infrastructure blocker, not
certification evidence.

The evidence collector now records the kernel release and SHA-256 of the
tested CLI, and requires a complete application/toolkit version map before
certifying a scenario matrix. It also reads `daemon_control_schema`
independently from the CLI JSON `schema_version`; the previous script compared
those unrelated versions and rejected every real daemon status as stale. The
version map and driver results remain runner-provided evidence to check
against the actual session, and no real-session route has passed the matrix.

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

The default-deny condition now has an end-to-end regression in the private
KWin and Sway harnesses and is a required `doctor --certify` report check. On
KWin, the isolated test client retained the trigger while the daemon reported
`compatibility-mode-off`, with zero uncertain output. The full doctor report
contract passed on clean commit `b2037049`. This validates opt-out refusal;
it does not establish integrity when a user explicitly enables compatibility
mode or verify the physical evdev monitor. The evdev hotplug monitor now polls
its topology watch before translating each retained raw-event slice, so a
large already-read batch cannot postpone a device-change interruption. Unit
coverage exercises this ordering; physical-device behavior remains unverified.

Acceptance criteria:

- A route with neither atomic replacement nor a reliable target-text check
  refuses automatic replacement unless the user or administrator explicitly
  enables its documented compatibility mode.
- A focus change, partial insertion, timeout, or lost acknowledgement cannot
  cause blind replay or rollback against changed text.
- Regression tests cover each transaction terminal state and the physical
  input interruption signal; real-session tests verify the client document
  after interruption.

### P1 — Restrict daemon control-socket mutations

**Status:** Managed read-only mode implemented; per-application authorization
remains open.

Linux control connections now validate `SO_PEERCRED` UID against the daemon.
This prevents a different Unix user from invoking daemon requests but does not
identify individual applications under the same UID. The root-owned
`/etc/wayexpand/policy.toml` can set
`[organization] disable_control_socket_mutations = true` to deny all
state-changing socket requests while retaining read-only diagnostics. This is
a mitigation for managed deployments, not a capability system: a same-UID
process may still use other user-session privileges, and focus tokens remain
public freshness values.

Acceptance criteria:

- Reject peer UIDs other than the daemon UID before parsing or dispatching
  requests; validate the Linux credential structure length.
- In restricted mode, deny insert, insert-target, reload, pause, resume, stop,
  and usage-stat clearing, while preserving status, focus, and explain.
- Prove that reading an exact focus token does not authorize an insert in the
  restricted mode, and test peer UID mismatch handling.
- Keep the remaining same-UID authorization limitation explicit. A stronger
  per-application approval mechanism requires a trusted OS authorization
  principal or user-confirmed capability flow and is not satisfied by a
  caller-supplied focus token.

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

The declared Rust 1.92 MSRV is now checked with explicit `cargo +1.92.0`
commands, avoiding the repository's development-toolchain override. The GUI
dependency is pinned to the compatible egui/eframe 0.35 line. Exact-head CI for
`584e38c6a4cb` passed all 33 jobs on 2026-10-31, including Rust 1.92 MSRV,
workspace tests and Clippy, native aarch64, Debian/RPM artifacts, Ubuntu 26.04
and Fedora package-install smoke tests, and headless KWin/Sway checks. Launchpad
sync also passed for that commit. This is release-candidate CI evidence only;
it does not close the real-session compositor-certification gate.

Partial evidence: hermetic per-user and release-archive installer tests cover
staged installation, validation preflight, rollback after filesystem, signal,
service restart, and service enable failures, immutable payload checks, and
custom XDG roots. The local x86_64 release archive smoke test also builds the
binary archive, verifies its checksum and required payload, and exercises the
packaged CLI. Binary archive assembly is byte-identical across differing
caller umasks on this host. These do not validate arm64 runtime behavior,
independent compiler reproducibility, package-manager install/upgrade, actual
service behavior, or published artifact signing/provenance on supported
release hosts. Re-ran `scripts/test-install-release.sh`,
`scripts/test-install-user.sh`, and `scripts/test-release.sh` on the current
checkout; all passed. This environment has no local container runtime for
package smoke tests; exact-head CI for `584e38c6a4cb` passed both
`Package Install Smoke (ubuntu:26.04)` and `Package Install Smoke
(fedora:latest)`, plus native aarch64 and Debian/RPM artifact jobs. The package
smoke script also refuses to run on a host without a Docker or
Podman marker, even when its explicit opt-in is set; a regression covers this
guard before any package-manager operation.

Acceptance criteria:

- Run the locked workspace check and release tests with the declared MSRV
  toolchain explicitly selected; do not allow `rust-toolchain.toml` to mask
  the tested compiler version.
- Build locked, reproducible artifacts for supported architectures and package
  formats; validate signatures and provenance.
- Pass package install, upgrade, rollback, and uninstall smoke tests on each
  supported distribution.
- Keep package availability, backend certification, and support status
  separate in release metadata. Do not publish a production release until P0
  gates pass.

### P2 — Complete per-action broker isolation

**Status:** Filesystem isolation, executable identity pinning, optional network
socket restrictions, and a restricted device-access profile are implemented
and regression-tested. A private device namespace and broader supported-host
verification remain open.

The official [`systemd.exec(5)` documentation](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#PrivateDevices=)
states that `PrivateDevices=` was added in systemd 239 and, in per-user
managers, implicitly enables `PrivateUsers=` and therefore requires working
unprivileged user namespaces. On this system (systemd 259), a transient
user-service probe failed with `218/CAPABILITIES`, and `unshare -Ur` was denied.
This environment cannot validate the setting. Do not add it unconditionally;
test it on supported non-containerized user managers and make setup fail
clearly or fall back to the documented restricted profile when unavailable.

Acceptance criteria:

- Validate the configured executable, open and pin its inode, and launch that
  pinned object; replacing the pathname after validation must not change what
  executes.
- Enforce each action's filesystem grants independently with Landlock, and
  fail closed when a requested sandbox cannot be enforced.
- Keep the network claim precise: the per-action filter rejects IP/packet/
  netlink socket creation and io_uring setup, while AF_UNIX remains available;
  it cannot prevent a same-UID local service from passing a network descriptor.
  The packaged systemd unit continues to deny IP networking globally.
- Keep device claims precise: the opt-in profile excludes `/dev` from explicit
  grants, denies `ioctl`, Unix-socket descriptor receipt, and device-node
  creation, and rejects inherited output. It is not a device namespace and
  does not classify device nodes nested inside granted directories.
- Add systemd `PrivateDevices=yes` (or an equivalent per-action device
  namespace) before claiming complete device isolation. First establish the
  minimum supported systemd version and user-namespace requirements, then
  verify startup and ordinary broker IPC on each supported user manager. Do
  not make the directive mandatory based only on `systemd-analyze verify`.
- Verify sandbox behavior and cancellation on supported kernels, and document
  that arbitrary malicious processes running under the broker's same UID are
  outside the IPC trust boundary.

### P2 — Reduce sensitive-data residency in process memory

**Status:** Best-effort wiping added for engine-owned working buffers; full
process-memory erasure is not achievable with the current public result and
configuration ownership model.

The engine wipes its rolling typed-character history, Unicode matching
scratch, invalidated undo records, dropped command-cache values, and deferred
reservations. It avoids making undo copies when undo is not configured. The
daemon wipes prefetched clipboard values on consumption, replacement,
expiration, and shutdown. Shipped systemd services disable core dumps with
`LimitCORE=0`.

Acceptance criteria:

- Exercise backspace, rolling-buffer eviction, reset, sensitive-focus change,
  Unicode scratch reuse, undo invalidation/commit, clipboard expiration, and
  engine shutdown; inspect designated allocations to confirm overwrite before
  logical clear or deallocation.
- Keep the rolling input limit and clipboard prefetch age bounded, and avoid
  retaining undo content when no undo chord is configured.
- Document that immutable config/results, in-flight command/template data,
  stack/register copies, allocator reallocation history, GUI processes outside
  systemd, and operating-system copies are outside this zeroization contract.
- Do not claim universal memory erasure or resistance to a privileged process
  dump based on these targeted wipes.

### P1 — Keep diagnostic errors free of private context

**Status:** Raw focus tokens and direct command stderr are no longer written to
routine logs or retained in the public command error.

Acceptance criteria:

- A focus-mismatch refusal log contains a fixed reason and message but neither
  expected nor actual compositor window token.
- A command that writes a recognizable secret to stderr and exits unsuccessfully
  yields a structured exit-status error whose `Display` and `Debug` output
  contain no child stderr; stderr remains drained so pipe capacity cannot block
  the child.
- Preserve privacy-safe failure categories for the dashboard and support trace.

### P2 — Close measured performance and internationalization gaps

**Status:** Open.

Partial evidence: the release-mode allocation probe in
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) measures zero allocations across
100,000 warmed nonmatching ASCII scalar events in each of the immediate and
deferred core paths. It excludes XKB/backend decoding, queueing, output, and
match-result construction, so it does not satisfy the end-to-end latency or
large-library profiling gates.

The 2026-10-30 GUI benchmark run measured trigger-conflict lookup at about
28 ns and draft edit plus preview at about 15 µs across 100 to 10,000 snippets.
The full validated save pipeline ranged from 3.19 ms at 100 snippets to 375 ms
at 10,000; it runs on the GUI's bounded background worker. Results and host
metadata are recorded in `docs/BENCHMARKS.md`. This is a short host-specific
Criterion run, not event-to-insertion or supported-hardware acceptance
evidence.

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
