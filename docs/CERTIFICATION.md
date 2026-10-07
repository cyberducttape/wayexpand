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

The JSON record includes the detected desktop, automatic input selection, the
currently connected daemon route (reported separately from that selection),
every check with a stable status, and the limitations of the selected backend.
The daemon route is a point-in-time status probe, not proof of correct typing
or insertion. The report is safe to attach to a support report and contains no
typed text or expansion contents. Certification scope is direct keyboard
input and already-committed text. Active IME/preedit composition is
intentionally out of scope and is reported as `unsupported-by-design`; a
certification never implies otherwise.

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
metadata and one result for every scenario × layout × client cell, but remains
`certified: false` unless every cell is explicitly passed, the doctor
snapshot reports `healthy: true`, and the selected backend probe is consistent
with the evidence metadata, including an exact match between the requested
compositor and the desktop identity returned by `doctor --json`. Daemon-backed
paths additionally require a status snapshot with `response: "running"`, the
current status schema, and the matching source/backend pair; IBus uses the
doctor IBus-installation probe because it is not the daemon control socket
path. Its stable `status` field is `certified`, `incomplete`, or `failed`.

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
whose required cells are all `pass` can set `certified: true`; human
support tables are compatibility observations, not certification evidence.

The collector writes its report in either format, then exits nonzero whenever
certification is incomplete or failed. Automation should still inspect
`.certified` and `.status` so it can distinguish an ineligible route from
missing scenario evidence; the JSON artifact remains available for upload and
review after a nonzero exit.

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
by the runner. The repository does not yet provide that real-client driver;
fake-driver contract tests verify orchestration only and are not compositor
evidence. Until real driver implementations run and reviewed artifacts pass,
every desktop remains uncertified.

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
drivers. A layout profile is evidence metadata only until the driver records
the actual keymap and observed behavior.

The workflow's backend column identifies the path a runner driver exercises;
it does not mean that path is currently eligible for production certification.
At present, IBus and both evdev paths are explicitly ineligible because they
cannot meet the safety contract. The workflow therefore runs
input-method-v2 as the production-required path on KDE and GNOME, while Sway
and Hyprland remain compatibility-only candidate jobs. Input-method-v2 is
still ineligible until the live daemon demonstrates the complete capability
contract above. Therefore the workflow is a fail-closed candidate-path test,
and its release gate requires durable certification evidence only for routes
that can satisfy the production safety contract. No desktop is production-
certified until its required evidence exists and passes.

| Environment | Current workflow candidate | Certification eligibility |
| --- | --- | --- |
| KDE Plasma / KWin | input-method-v2 (production-required); KWin window tracker available for app-filter scenarios | Production-required candidate; requires live local dead-key/Compose tracking and the full input/output contract |
| GNOME | input-method-v2 (production-required); IBus remains a compatibility candidate | Input-method-v2 candidate; requires the complete live capability contract |
| Sway | evdev plus wlroots virtual keyboard (compatibility-only) | Ineligible for production certification: evdev lacks sensitive-field awareness and atomic replacement |
| Hyprland | evdev plus wlroots virtual keyboard (compatibility-only) | Ineligible for production certification: evdev lacks sensitive-field awareness and atomic replacement |

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
  password-field awareness is unavailable.
- **Experimental** opts into input-method-v2 and clearly identifies its
  unsupported non-text-key and IME/preedit behavior.

Experts can inspect the underlying protocol decision with
`wayexpand explain-backend`. Administrators may continue to select exact
daemon source/backend arguments in service configuration.

## Self-hosted workflow

The `.github/workflows/certification.yml` workflow runs the four target
desktops as a matrix on manual dispatch and weekly schedule. Each runner must provide an
executable `WAYEXPAND_CERTIFICATION_DRIVER` and set
`WAYEXPAND_CERTIFICATION_COMPOSITOR`, `WAYEXPAND_CERTIFICATION_VERSION`,
`WAYEXPAND_CERTIFICATION_LAYOUT`, and `WAYEXPAND_CERTIFICATION_TARGET_APPS`.
Missing driver or session metadata fails the job; hosted CI is never treated as
a substitute for a real compositor session. Workflow artifacts are evidence
records only; the support matrix should not mark a compositor certified until a
reviewed artifact reports `certified: true` for the relevant compositor,
backend, layout, and client set.
