# Desktop certification

`wayexpand certify` is a local, machine-readable compatibility report. It
separates protocol availability from evidence gathered by real client tests.
An available protocol is not a certification: the report remains `NOT
CERTIFIED` while typing, focus, password-field, restart, and reload scenarios
are `not-run`.

Use:

```text
wayexpand certify
wayexpand certify --json > wayexpand-certification.json
```

For a reproducible live output check, run:

```text
wayexpand doctor --certify
wayexpand doctor --certify --json > wayexpand-smoke-report.json
```

This starts a private headless KWin session, launches isolated Wayland sink
clients, and checks the text those clients actually receive through the
production daemon and libei output backend. It checks a basic trigger, focus
switching between test windows, word-boundary delimiters, cursor placement,
multiline output, command completion, configuration reload, daemon restart,
and safe refusal of an unavailable character. All these scenarios must pass
for the report to say `partial`. When KWin offers the negotiated `ei_text`
route, it also inserts a replacement containing accented Latin and Japanese
text and verifies the exact contents received by the test client. A keysym-only
route reports Unicode insertion as not tested; it never infers Unicode support
from capability metadata. The private harness simulates keyboard capture
through stdin; it does not exercise the user's active compositor. Its concise
report therefore says `partial`,
and keeps keyboard capture, suspend/resume, and full production certification
unverified. The report records the negotiated insertion mode, backend detail,
capture source, and sensitive-field signal observed from the private daemon.
This makes keysym fallback and its pacing visible alongside the insertion
results; it does not turn a reported capability into a tested guarantee.
Source checkouts require Rust
build tools, KWin, D-Bus session tools, `jq`, and `timeout`; binary packages
ship the isolated test helpers, but still require KWin and the session tools.
The harness uses a temporary home/config/runtime directory and does not
control the active desktop or its daemon.

The JSON record includes the detected desktop, automatic input selection, the
currently connected daemon route (reported separately from that selection),
every check with a stable status, and the limitations of the selected backend.
The isolated smoke report also records the commit of the CLI and daemon
binaries exercised. A mismatch with the reporting CLI fails the evidence
check, preventing a stale source-checkout CLI from claiming a partial pass for
binaries rebuilt from a newer checkout.
The active route is verified only when its status schema and `daemon_commit`
match the CLI that produced the report; a stale daemon is reported as
unavailable and must be restarted or updated first. A build without Git
metadata reports its commit as `unknown`, which cannot show a match either;
set `WAYEXPAND_BUILD_SHA` when building packages from source archives. Debian and
Launchpad source publishers stamp this automatically. The daemon route remains a
point-in-time status probe, not proof of correct typing or insertion. The
report is safe to attach to a support report and contains no typed text or
expansion contents. Certification scope is direct keyboard
input and already-committed text. Active IME/preedit composition is
intentionally out of scope and is reported as `unsupported-by-design`; a
certification never implies otherwise.

The latest current-source smoke report at
[`docs/evidence/doctor-certify-a09536c13fa7.json`](evidence/doctor-certify-a09536c13fa7.json)
tested commit `a09536c13fa7` on KWin 6.6.6 in a private headless session. Eleven
insertion, focus, reload, restart, and refusal checks passed; the build identity
also matched. Unicode, physical keyboard capture, and suspend/resume were not
tested. Twenty libei keysym-fallback transactions per size measured injector
p50/p95/p99 of 12.562/12.627/12.633 ms (small), 784.175/787.050/787.390 ms
(medium), and 3,146.976/3,155.291/3,155.805 ms (large). Capture was simulated
through stdin. This is partial compatibility evidence, not production
compositor certification.

The previous current-source smoke report at
[`docs/evidence/doctor-certify-d776185d5039.json`](evidence/doctor-certify-d776185d5039.json)
tested commit `d776185d5039` on KWin 6.6.6 in a private headless session. Eleven
insertion, focus, reload, restart, and refusal checks passed; the build identity
also matched. Unicode, physical keyboard capture, and suspend/resume were not
tested. Twenty libei keysym-fallback transactions per size measured injector
p50/p95/p99 of 12.546/12.594/12.601 ms (small), 783.531/785.375/786.330 ms
(medium), and 3,151.121/3,158.146/3,160.430 ms (large). Capture was simulated
through stdin. This is partial compatibility evidence, not production
compositor certification.

The previous current-source smoke report at
[`docs/evidence/doctor-certify-d5153cb7a2a3.json`](evidence/doctor-certify-d5153cb7a2a3.json)
tested commit `d5153cb7a2a3` on KWin 6.6.6 in a private headless session.
Twelve output, focus, reload, restart, and refusal checks passed; one latency
measurement was recorded and three checks were not tested. Twenty libei
keysym-fallback transactions per size measured injector p50/p95/p99 of
12.531/12.570/12.602 ms (small), 784.552/788.881/789.245 ms (medium), and
3,146.258/3,156.005/3,157.177 ms (large). Capture was simulated through
stdin; Unicode insertion, physical keyboard capture, sensitive-field behavior,
suspend/resume, and long-running stability remain untested. This report is
partial compatibility evidence, not production compositor certification.

The current-source smoke report at
[`docs/evidence/doctor-certify-f34ce2bdcdeb.json`](evidence/doctor-certify-f34ce2bdcdeb.json)
tested build `f34ce2bdcdeb` on KWin 6.6.6 in a private headless session. Twelve
checks passed, one backend latency measurement was recorded, and three checks
were not tested. Twenty small libei keysym-fallback transactions measured
12.536 ms p50 and 12.646 ms p95/p99 inside the injector. The run verified its
CLI/daemon build identity and clean source tree. Capture was simulated through
stdin; Unicode insertion, physical keyboard capture, sensitive-field behavior,
suspend/resume, and long-running stability remain untested. This is partial
compatibility evidence, not production compositor certification.

The current-source smoke report at
[`docs/evidence/doctor-certify-e4d0d11eb69b.json`](evidence/doctor-certify-e4d0d11eb69b.json)
tested build `e4d0d11eb69b` on KWin 6.6.6 in a private headless session. Twelve
checks passed and three were not tested. Twenty libei keysym-fallback
transactions per size measured p50/p95/p99 of 12.529/12.630/12.668 ms (small),
781.704/785.215/785.405 ms (medium), and 3,141.102/3,150.865/3,151.163 ms
(large). The report verified build identity and a clean source tree. Capture
was simulated through stdin; Unicode insertion, physical keyboard capture,
suspend/resume, sensitive-field behavior, and long-running stability remain
untested. This is partial compatibility evidence, not production compositor
certification.

The latest preserved smoke report at
[`docs/evidence/doctor-certify-b9fdc52f3d7d.json`](evidence/doctor-certify-b9fdc52f3d7d.json)
tested commit `b9fdc52f3d7d` on KWin 6.6.6 in a private headless session. It
passed 12 output, focus, reload, restart, and refusal checks; Unicode insertion,
keyboard capture, and suspend/resume were not tested. The negotiated route was
libei keysym fallback with stdin-simulated capture and no sensitive-field
signal. Twenty samples per size measured injector p50/p95/p99 of 12.519/12.555/
12.565 ms (small), 779.543/781.033/781.814 ms (medium), and
3,127.174/3,132.945/3,134.460 ms (large). These transactions exclude physical
capture and client paint latency and are measurements, not SLO results. The
report confirms the tested build identity and clean certification inputs; it
does not certify a production desktop route.

The current-source private KWin E2E report at
[`docs/evidence/kwin-e2e-71d6f1e43.txt`](evidence/kwin-e2e-71d6f1e43.txt)
tested commit `71d6f1e4397c` on KWin 6.6.6. Twelve real-client output and
focus scenarios passed, including focus changes during a command-backed
expansion, client closure, reload, restart, and output latency. The production
libei output backend and KWin focus tracker were exercised; keyboard capture
was still simulated through stdin. The keysym fallback refused an unavailable
character and did not test full Unicode insertion. This is fresh compatibility
evidence, not production certification.

The preserved current-build smoke report at
[`docs/evidence/doctor-certify-bc3b5e1b88f8.json`](evidence/doctor-certify-bc3b5e1b88f8.json)
tested commit `bc3b5e1b88f8` on KWin 6.6.6. Twelve checks passed, one latency
sample was measured, and three checks were not tested. With libei keysym
fallback, 20 small replacements measured p50 12.529 ms and p95/p99 12.619 ms.
This remains partial evidence: it does not test physical keyboard capture,
sensitive-field behavior, suspend/resume, or long-running stability.

The current-source report at
[`docs/evidence/doctor-certify-2bac9c84aeb5.json`](evidence/doctor-certify-2bac9c84aeb5.json)
tested commit `2bac9c84aeb5` on KWin 6.6.6 in a private headless session.
Twelve checks passed, one latency sample was measured, and three checks were
not tested. Twenty small libei keysym-fallback transactions measured p50
12.456 ms and p95/p99 12.592 ms. Capture was simulated over stdin; this does
not establish physical keyboard capture, sensitive-field behavior, or
production compositor certification.

For an operator evidence record after running the real-client scenarios, use
the collector with `--format json` and a validated results file:

```sh
scripts/certify-compositor.sh --format json --compositor kde \
  --version 6.6.2 --backend input-method-v2 \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit \
  --results kde-results.txt --output kde-certification.json
```

That record is distinct from the CLI preflight: it includes the exact session
metadata and one result for every scenario × layout × client cell. Schema 3
separates `certified` from `certification_scope`: a compatibility run can pass
every scenario and report `certified: true` with scope `compatibility`, but
only scope `production` can satisfy the release gate. Production scope also
requires a production-eligible route and a matrix target explicitly marked
production-required. The report remains `certified: false` unless every cell
is explicitly passed, the doctor snapshot reports `healthy: true`, and the
selected backend probe is consistent with the evidence metadata, including an
exact match between the requested compositor and the desktop identity
returned by `doctor --json`. Daemon-backed paths additionally require a status
snapshot with `response: "running"`, the current status schema, and the
matching source/backend pair; IBus uses the doctor IBus-installation probe
because it is not the daemon control socket path. `status` reports scenario
completeness (`certified`, `incomplete`, or `failed`); `certification_scope`
reports compatibility or production evidence.

Raw evdev capture is a compatibility fallback and is never eligible for a
production certification: it has no sensitive-field signal and cannot provide
atomic replacement. The collector records this as
`backend_certification_eligible: false` even when a driver reports all
functional cells as passing. This prevents a successful best-effort evdev run
from being presented as a safe desktop certification.

The IBus route is also not eligible for production certification: its separate
delete/commit operations are not atomic, it has no portable exact window
identity, and it does not observe active composition. Live GTK/Qt scenario
passes cannot compensate for those missing guarantees.

The same collector requires the live input-method route to report local
dead-key/Compose tracking, sensitive-field handling, key pass-through, atomic
replacement, and full Unicode support. It does not require awareness of an
external IME's active preedit: that remains explicitly unsupported-by-design
and outside this certification's scope. The status contract keeps these
guarantees separate (`capture_local_compose_aware` versus
`capture_composition_aware`) so certification of committed text cannot be
misread as support for expansion during active IME composition.

Matrix-cell outcomes are deliberately more expressive than pass/fail: `pass`
means the scenario passed, `fail` means it was exercised and failed,
`unsupported-by-design` records a documented capability that the selected
backend cannot provide (for example password-field awareness through evdev),
and `UNVERIFIED` means no trustworthy result was collected. Only an artifact
whose required cells are all `pass` can set `certified: true`; the separate
`certification_scope` field determines whether this is compatibility or
production evidence. Human support tables are compatibility observations,
not certification evidence.

The collector writes its report in either format, then exits nonzero whenever
certification is incomplete or failed. Automation must inspect
`.certified`, `.status`, and `.certification_scope`; a compatibility-scoped
pass is not a production pass. The JSON artifact remains available for upload
and review after a nonzero exit.

The compositor evidence collector emits schema version 2, where each entry in
`scenarios` is an individual scenario × layout × client result. Scenario-only
schema-1 artifacts do not prove this expanded matrix.
The artifact also carries `out_of_scope_capabilities`; these are explicit
product boundaries, not untested passing scenarios. The committed-text
composition scenarios exercise expansion after the user completes composition.

The KDE self-hosted workflow also runs the live KWin tracker lifecycle test,
which requires a non-empty compositor-issued identity for the focused window,
rejects an untrusted D-Bus focus event, and detects when its own temporary KWin
script is unloaded. This is a focused backend integration check, not a typing
or end-to-end certification result.
The workflow still expects a compositor-specific executable driver configured
by the runner. Fake-driver contract tests verify orchestration only and are
not compositor evidence. Until real driver implementations run and reviewed
artifacts pass, every desktop remains uncertified.

### Headless KWin real-client driver

`scripts/kwin-certification-driver.sh` is a driver for the KDE target's
`evdev+libei` path. Run the slow part once, then hand the store to the
runner:

```sh
scripts/kwin-certification-prepare.sh --store kwin-store.txt
WAYEXPAND_KWIN_CERTIFICATION_STORE=$PWD/kwin-store.txt \
scripts/run-certification-driver.sh --driver scripts/kwin-certification-driver.sh \
  --compositor kde --version 6.6.6 --backend evdev+libei \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk3-textview,qt6-plaintextedit,terminal-konsole,password-field,text-editor-gedit,qt6-editor-kate,browser-firefox,electron-vscode \
  --output kde-results.txt
```

For each layout profile the prepare step starts a private KWin with its own
D-Bus session, accessibility bus, home and keyboard layout (`us`, `de`, `fr`,
`us(altgr-intl)` for `altgr`, and `us,de` with a layout switch between the
two halves of two-phase scenarios for `multi-layout-switching`). It runs the
production daemon, libei output and KWin tracker against real clients, and
each client reports the text it actually received:

| Target app | Client | Read back through |
|---|---|---|
| `gtk3-textview` | GTK 3 `TextView` | the client's own state file |
| `password-field` | GTK 3 password-purpose `Entry` | the client's own state file |
| `qt6-plaintextedit` | Qt 6 `QPlainTextEdit` | the client's own state file |
| `terminal-konsole` | Konsole with a raw-mode recorder | bytes the terminal sent to its pty |
| `text-editor-gedit` | gedit | AT-SPI |
| `qt6-editor-kate` | Kate | AT-SPI |

Every case starts its client with empty settings so that session restore
cannot bring back an earlier document. The driven scenarios are
`multiline-rapid`, `focus-cross-window`, `focus-change-during-expansion`,
`target-closes-during-expansion`, `config-reload`, `daemon-restart`,
`daemon-restart-during-typing`, `failed-insertion`, `unicode-combining`,
`command-timeout`, `expansion-cancellation`, and `text-editor-client`.

The driver answers a cell from the store only if the store records the build
commit of the CLI under certification (`WAYEXPAND_CLI`), the KWin version
being certified, and a clean source tree. The following cells are
`UNVERIFIED` whatever the store says: anything that needs physical keyboard
capture (the private daemon reads its keys from stdin), key pass-through,
the pending-output scenarios `focus-change-during-expansion`,
`target-closes-during-expansion` and `expansion-cancellation` (evdev capture
hands results to its serialized output pipeline and its key-release gating,
which stdin capture never reaches; the driver log keeps the stdin
observation),
input hardware, suspend/resume, compositor restart, the RemoteDesktop portal
(the session reaches EIS through a relay), the snippet picker, and clients
the store does not cover. That includes Firefox and Chromium, whose snap
packages cannot join a private compositor, and Electron. `password-field` is
`unsupported-by-design`, because evdev capture has no sensitive-field signal.
The IBus path is not exercised. A run of this driver can therefore never
certify KDE. It supplies reproducible real-client evidence for the output,
focus and daemon-lifecycle part of the matrix, and leaves the rest explicitly
open. `scripts/test-kwin-certification-driver.sh` checks the driver's
contract without a compositor.

Each driver invocation is capped at 120 seconds and the complete matrix has a
60-minute deadline. A timed-out cell and every cell remaining after the overall
deadline are recorded as `UNVERIFIED`; the collector therefore cannot turn a
hung driver into a pass or lose the evidence for cells that were not run.

The checked-in target and scenario contract is
[`tests/certification/compositor-matrix.json`](../tests/certification/compositor-matrix.json).
CI validates that all four required desktop targets and all forty-five scenarios
remain present, rejects evidence that pairs a compositor with a backend
outside its declared certification paths, and requires the layout profiles
`us`, `de`, `fr`, `altgr`, and `multi-layout-switching` from certification
drivers. Each target's `certification_backend` and
`production_certification_required` fields also drive workflow backend
selection and required runner checks; production targets must select the
input-method-v2 route. A layout profile is evidence metadata only until the
driver records the actual keymap and observed behavior.

## Headless real-client evidence (not certification)

[`evidence/kwin-real-client-e075ddda6.txt`](evidence/kwin-real-client-e075ddda6.txt)
records the 1,800-cell compatibility matrix produced on 2026-10-27 by the
KWin 6.6.6 private real-client driver, using build `e075ddda6ada`. Its
[summary](evidence/kwin-real-client-e075ddda6.summary.json) records 187
passing cells, 40 password-field cells marked `unsupported-by-design`, 1,573
unverified cells, and no failed cells. The collector reported
`certified: false`, `status: incomplete`, and compatibility scope. Capture was
simulated through stdin, the active daemon status probe was unavailable, and
the evdev route has no sensitive-field signal or atomic replacement. These
results are compatibility evidence only and do not certify KDE or a production
keyboard-capture route.

[`evidence/kwin-real-client-c8c27d86d632.txt`](evidence/kwin-real-client-c8c27d86d632.txt)
is the store from a clean-tree run of
`scripts/kwin-certification-prepare.sh` at commit `c8c27d86d632` on KWin
6.6.6, recorded 2026-10-27. This is an earlier archived run; the later
`e075ddda6ada` matrix above supersedes its driver results. The archived run has
[`evidence/kwin-real-client-c8c27d86d632.summary.json`](evidence/kwin-real-client-c8c27d86d632.summary.json)
summarizes the collector report built from it. The live doctor and daemon
snapshots are left out because they describe the reporting workstation.

In the four single-layout sessions (`us`, `de`, `fr`, `us(altgr-intl)`), 258
of 288 driven cells passed and none failed across GTK 3, Qt 6, Konsole, a
GTK password field, gedit and Kate. The 30 unverified cells are
`unicode-combining` (the keysym route refuses combining marks cleanly but
cannot show inserting them), `multiline-rapid` in the single-line password
field, and two Kate `expansion-cancellation` cells. Those two came from a
harness ordering race, which has since been fixed. In the
`multi-layout-switching` session nothing could be typed, by the daemon or by
the test typist, because KWin never reported the active layout (see
[SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)). All 72 of its cells are unverified.

That earlier run's `run-certification-driver.sh` matrix had 188 pass, 40
`unsupported-by-design` (password-field scenario: evdev has no sensitive-field
signal) and 1,572 `UNVERIFIED`. The updated run above has the current counts.
Both reports are incomplete compatibility evidence. Capture is simulated,
browsers and Electron are not driven, and the pending-output scenarios do not
count for the evdev route. KDE remains uncertified.

## Supporting component evidence (not certification)

[`evidence/kwin-tracker-live-2026-10-26.json`](evidence/kwin-tracker-live-2026-10-26.json)
records a focused test against the active KDE Plasma Wayland session on KWin
6.6.6, using commit `d01033979da6`. The tracker received an exact focused
window identity, rejected an untrusted D-Bus focus event, and detected
unloading its temporary KWin script. This covers the window-tracker component
only; it does not exercise keyboard capture, text insertion, client behavior,
or the compositor certification matrix.

## Supporting headless evidence

[`evidence/doctor-certify-897eeecaaef7.json`](evidence/doctor-certify-897eeecaaef7.json)
preserves the build-identity-checked `wayexpand doctor --certify --json` run
from commit `897eeecaaef7` on KWin 6.6.6. Twelve output, focus, reload,
restart, refusal, and build-identity checks passed. The libei keysym fallback
measured 12,536 µs p50 and 12,575 µs p95/p99 across twenty small insertions.
Keyboard capture was simulated through stdin; Unicode insertion and
suspend/resume were not tested. This partial report does not certify a
production compositor route.

[`evidence/doctor-certify-061c3d366a9d.json`](evidence/doctor-certify-061c3d366a9d.json)
preserves the build-identity-checked run from commit `061c3d366a9d` on KWin
6.6.6. Ten isolated-client checks passed for insertion, focus transitions,
reload, restart, and refusal; the keysym fallback measured 12,523 µs p50,
12,607 µs p95, and 12,607 µs p99 for twenty small insertions inside the
injector. Unicode insertion, physical keyboard capture, and suspend/resume
were not tested. This partial headless evidence does not certify the active
desktop or production capture route.

[`evidence/doctor-certify-a4d699546.json`](evidence/doctor-certify-a4d699546.json)
preserves a current-source run from commit `a4d69954676e` on KWin 6.6.6.
The isolated clients passed ten insertion, focus, reload, restart, and refusal
scenarios. The keysym fallback measured 12,521 µs p50, 12,827 µs p95, and
12,827 µs p99 across twenty small insertions inside the injector. Unicode
insertion, physical keyboard capture, and suspend/resume were not tested. This
is partial headless evidence, not production compositor certification.

[`evidence/doctor-certify-6551bb4b7.json`](evidence/doctor-certify-6551bb4b7.json)
preserves the build-identity-checked `wayexpand doctor --certify --json` run
from commit `6551bb4b7b77` on KWin 6.6.6. The private clients passed 12
output, focus, configuration reload, daemon restart, refusal, and build
identity checks. The route used libei keysym fallback with 12 ms pacing;
Unicode insertion, physical keyboard capture, and suspend/resume were not
tested. This partial result supports the headless output/focus path only and
does not certify the active desktop or production keyboard-capture route.

[`evidence/doctor-certify-bdf55e979.json`](evidence/doctor-certify-bdf55e979.json)
preserves the build-identity-checked `wayexpand doctor --certify --json` run
from commit `bdf55e97993c` on KWin 6.6.6. The private clients passed 12
output, focus, configuration reload, daemon restart, and refusal checks. The
route used libei keysym fallback with 12 ms pacing; Unicode insertion, physical
keyboard capture, and suspend/resume were not tested. This partial result
supports the headless output/focus path only and does not certify the active
desktop or production keyboard-capture route.

[`evidence/doctor-certify-3e3ea129d7a5.json`](evidence/doctor-certify-3e3ea129d7a5.json)
preserves a `wayexpand doctor --certify --json` report from commit
`3e3ea129d7a5` on KWin 6.6.6. The isolated clients passed 12 output,
focus, configuration reload, daemon restart, and refusal checks. The route
used libei keysym fallback with 12 ms pacing; Unicode insertion, physical
keyboard capture, and suspend/resume were not tested. This partial result
supports the headless output/focus path only and does not certify the active
desktop or production keyboard-capture route.

[`evidence/doctor-certify-552f1c137.json`](evidence/doctor-certify-552f1c137.json)
preserves the build-identity-checked `wayexpand doctor --certify --json` run
from commit `552f1c1377eb` on KWin 6.6.6. The isolated clients passed 12
insertion, focus, safety, and build-identity checks. The route used libei
keysym fallback with 12 ms pacing; Unicode insertion, physical keyboard
capture, and suspend/resume were not tested. This remains headless output and
focus evidence, not desktop certification.

[`evidence/doctor-certify-2111dba66.json`](evidence/doctor-certify-2111dba66.json)
preserves a current-source run of `wayexpand doctor --certify --json` from
commit `2111dba66512` on KWin 6.6.6. The private clients passed 11 insertion,
focus, and safety checks. The negotiated route was libei keysym fallback with
12 ms pacing; Unicode insertion, physical keyboard capture, and suspend/resume
were not tested. This is repeatable headless output/focus evidence, not
certification of the active desktop or production keyboard-capture route.

[`evidence/doctor-certify-cb70c9ee3.json`](evidence/doctor-certify-cb70c9ee3.json)
preserves the 12-scenario `wayexpand doctor --certify --json` run from commit
`cb70c9ee303e` on KWin 6.6.6. The isolated clients passed insertion,
word-boundary, cursor, multiline, focus-switch, focus-change, target-close,
command-completion, configuration-reload, daemon-restart, and unavailable-
Unicode-refusal checks. Unicode insertion, physical keyboard capture, and
suspend/resume were not tested. The route used libei keysym fallback with
12 ms key pacing. This remains headless output/focus evidence and does not
certify the active desktop or production keyboard-capture route.

[`evidence/kwin-headless-99b4bdc50.json`](evidence/kwin-headless-99b4bdc50.json)
records an 11-scenario run against the production daemon, libei injector, and
KWin window tracker from commit `99b4bdc50` on KWin 6.6.6. All scenarios
passed, including focus change during expansion and target closure. Capture was
simulated through stdin in a private headless session; the route negotiated
keysym fallback, so Unicode insertion remains untested. This artifact supports
output and focus behavior only and does not certify a compositor route.

[`evidence/kwin-headless-2026-10-25.json`](evidence/kwin-headless-2026-10-25.json)
records a current-source 11-scenario run against the production daemon, libei
injector, and KWin window tracker on KWin 6.6.6. All scenarios passed,
including focus change during expansion and target closure. Capture was
simulated through stdin in a private headless session; the route negotiated
keysym fallback, so Unicode insertion remains untested. This artifact supports
output and focus behavior only and does not certify a compositor route.

[`evidence/doctor-certify-2026-10-25.json`](evidence/doctor-certify-2026-10-25.json)
is the current-source run of `wayexpand doctor --certify --json` from build
`c2d4a153a232` on KWin 6.6.6. The private test clients passed 12 insertion,
focus, reload, restart, refusal, and build-identity checks. The session
negotiated libei keysym fallback; its 20 small-insertion samples measured
12,447 µs p50, 12,591 µs p95, and 12,591 µs p99 inside the injector, excluding
capture and client paint. Unicode insertion, physical keyboard capture, and
suspend/resume remain explicitly untested. This partial report does not
describe or certify the active desktop.

[`evidence/doctor-certify-2026-10-26.json`](evidence/doctor-certify-2026-10-26.json)
records a clean-source run from commit `27b0a6c7db6b` on KWin 6.6.6. The
isolated clients passed all 12 exercised insertion, focus, reload, restart,
refusal, and build-identity checks. The route used libei keysym fallback with
12 ms pacing; 20 small replacements measured 12,504 µs p50, 12,581 µs p95,
and 12,581 µs p99 inside the injector. Unicode insertion, physical keyboard
capture, and suspend/resume remain untested. This is partial headless
compatibility evidence, not production certification or evidence about the
active desktop.

[`evidence/doctor-certify-1a274fbd6fa7.json`](evidence/doctor-certify-1a274fbd6fa7.json)
preserves the current-source report from commit `1a274fbd6fa7` on KWin 6.6.6.
The isolated clients passed 12 insertion, focus, reload, restart, refusal, and
build-identity checks. The session used libei keysym fallback with 12 ms
pacing; 20 small replacements measured 12,375 µs p50 and 12,531 µs p95/p99
inside the injector. Unicode insertion, physical keyboard capture, and
suspend/resume were not tested. This partial headless report does not certify
the active desktop or production capture route.

[`evidence/doctor-certify-0d1b2138a8fd.json`](evidence/doctor-certify-0d1b2138a8fd.json)
preserves the installed build's KWin 6.6.6 `doctor --certify --json` run for
commit `0d1b2138a8fd`. Twelve output, focus, reload, restart, refusal, and
build-identity checks passed. The route used libei keysym fallback; 20 small
injector transactions measured 12,198 µs p50 and 12,220 µs p95/p99. Unicode
insertion, physical keyboard capture, and suspend/resume were not tested. The
source-tree state is unknown for this installed-binary run. It is partial
headless compatibility evidence, not active-desktop or production certification.

[`evidence/doctor-certify-5ec3122de9de.json`](evidence/doctor-certify-5ec3122de9de.json)
preserves the current-source KWin 6.6.6 `doctor --certify --json` run for
commit `5ec3122de9de`. Twelve checks passed; Unicode insertion,
suspend/resume, and physical keyboard capture were not tested. Twenty small
libei keysym-fallback transactions measured 12,532 µs p50 and 12,579 µs
p95/p99 inside the injector. Capture was simulated through stdin, and the
session was private and headless. This is partial output/focus compatibility
evidence, not active-desktop or production certification.

[`evidence/doctor-certify-ff6eeeac8.json`](evidence/doctor-certify-ff6eeeac8.json)
preserves the current-source KWin 6.6.6 report for commit `ff6eeeac8d83`.
Twelve checks passed; Unicode insertion, suspend/resume, and physical keyboard
capture were not tested. It records 20 injector samples in each size bucket:
small p95 12,630 µs, medium p95 780,887 µs, and large p95 3,143,614 µs.
The benchmark raises the isolated test config's `keystroke_max_chars` to 260
to exercise a 257-character large sample; this does not change the product
default. Capture was simulated through stdin in private headless KWin. These
measurements exclude input capture and client paint and are not a production
SLO or certification.

[`evidence/kwin-headless-2026-10-24.json`](evidence/kwin-headless-2026-10-24.json)
records an 11-scenario run against the production daemon, libei injector, and
KWin window tracker on KWin 6.6.6. All 11 scenarios passed. The private KWin
session was headless and keyboard capture was simulated through stdin, so this
artifact supports the output and focus paths only. It does not certify a
compositor route and does not satisfy any real-session acceptance gate.

The workflow's backend column identifies the path a runner driver exercises;
it does not mean that path is eligible for production certification. Both
evdev paths lack sensitive-field awareness and atomic replacement. IBus
verifies surrounding text but is non-atomic and lacks exact window identity
and composition awareness. KWin and Mutter do not advertise
`zwp_input_method_manager_v2`, so input-method-v2 is not a valid target for
either desktop ([protocol support matrix](https://wayland.app/protocols/input-method-unstable-v2)). Sway and Hyprland are now declared as additional
input-method-v2 test targets because their compositor stacks can expose that
protocol. The live manager, seat, input context, capability contract, and
complete real-session scenarios must still be verified on each target; this
declaration is not a support or certification claim. No declared compositor
route currently meets the production contract. A release invocation therefore
fails closed before runner discovery until a compatible real-session route is
added. Manual and scheduled runs can still collect compatibility evidence from
online compositor runners; those reports cannot satisfy the release gate.

| Environment | Current workflow candidate | Certification eligibility |
| --- | --- | --- |
| KDE Plasma / KWin | IBus compatibility path is run when its labeled real-session runner is online | No eligible production route; input-method-v2 manager unavailable |
| GNOME | IBus compatibility path is run when its labeled real-session runner is online | No eligible production route; IBus replacement is non-atomic and has no window tracker |
| Sway | input-method-v2 is the current compatibility workflow candidate; evdev plus wlroots remains a declared fallback path | No production certification; the workflow does not yet require production evidence |
| Hyprland | input-method-v2 is the current compatibility workflow candidate; evdev plus wlroots remains a declared fallback path | No production certification; the workflow does not yet require production evidence |

The scenario contract still requires GTK and Qt clients, relevant password
fields, and focus isolation on each desktop. Application-filter scenarios
must verify KWin tracking on KDE; on GNOME, Sway, and Hyprland filters must
remain visibly unavailable. This coverage exercises candidate behavior and
does not override a route's missing capabilities.

Each environment must cover printable press/release, auto-repeat, modifiers,
Unicode and combining text, multiline replacement, password fields, focus
transitions, cross-window isolation, and picker handoff between two windows
with the same application ID and title. The picker scenario passes only when
selection after focusing the other window cannot insert into that wrong
window. Also cover configuration reload, daemon restart, compositor restart,
and failed insertion. Unsupported capabilities remain
explicit in the report; the harness must not convert an untested feature into
a pass. On backends marked “application filters unavailable”, configured app
filters must remain visibly unavailable rather than being inferred from
unreliable window metadata.

## Compatibility modes

Normal setup uses compatibility modes instead of exposing protocol names:

- **Recommended** chooses IBus when the installed component is discoverable,
  then the safest available local alternative. Availability is not an
  end-to-end guarantee; `wayexpand certify` remains the authority for that
  distinction.
- **Maximum compatibility** uses evdev with a detected libei/EIS path (or a
  KDE/GNOME portal candidate after explicit authorization) and warns that
  password-field awareness is unavailable. Its keystroke output cannot verify
  what it erases, so typed expansions stay refused until the user sets
  `compatibility_mode` (see [Insertion guarantees](../SECURITY.md#insertion-guarantees)).
- **Experimental** opts into input-method-v2 and clearly identifies its
  unsupported non-text-key and IME/preedit behavior.

Experts can inspect the underlying protocol decision with
`wayexpand explain-backend`. Administrators may continue to select exact
daemon source/backend arguments in service configuration.

## Self-hosted workflow

The `.github/workflows/certification.yml` workflow has two modes. Release
calls set `require_production: true` and fail closed unless each
production-required target yields reviewed production evidence. Manual and
scheduled runs default to compatibility evidence, which cannot satisfy the
release gate. Set the manual `require_production` input to true to exercise the
release gate directly.

Both modes need the `CERTIFICATION_RUNNER_TOKEN` Actions secret: a fine-grained
token with repository **Administration: read** permission, used only to
discover online runners and their labels. Register a real-session runner with
`wayexpand-certification` and its compositor label (`kde`, `gnome`, `sway`, or
`hyprland`). Compatibility targets without an online runner are skipped;
production-required targets without one fail preflight. Each runner must provide
an executable `WAYEXPAND_CERTIFICATION_DRIVER` and set
`WAYEXPAND_CERTIFICATION_COMPOSITOR`, `WAYEXPAND_CERTIFICATION_VERSION`,
`WAYEXPAND_CERTIFICATION_LAYOUT`, and `WAYEXPAND_CERTIFICATION_TARGET_APPS`.

As of 2026-10-28, the repository has no registered self-hosted runners and no
`CERTIFICATION_RUNNER_TOKEN` secret. Therefore, no automated real-session
certification job can currently run. The test suite and hosted headless jobs
remain useful but do not substitute for this evidence.

Missing driver or session metadata fails the job; hosted CI is never treated as
a substitute for a real compositor session. Runners certifying a keystroke
route (`evdev+libei`, `evdev+wlroots`) must give consent in the configuration
their driver uses: `compatibility_mode = "tracked-focus"` on KDE and
`"untracked-focus"` where no window tracker exists. Without it the daemon
reports `insertion_assurance=blocked` and refuses every typed scenario.
Workflow artifacts are evidence
records only; the support matrix should not mark a compositor certified until a
reviewed artifact reports `certified: true` for the relevant compositor,
backend, layout, and client set.
