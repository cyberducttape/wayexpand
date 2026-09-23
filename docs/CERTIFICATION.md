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

The JSON record includes the detected desktop, selected input mode, every
check with a stable status, and the limitations of the selected backend. It is
safe to attach to a support report and contains no typed text or expansion
contents.

For an operator evidence record after running the real-client scenarios, use
the collector with `--format json` and a validated results file:

```sh
scripts/certify-compositor.sh --format json --compositor kde \
  --version 6.6.2 --backend ibus --layout us \
  --target-apps gtk4-demo,qt6-demo,terminal,browser,password-field \
  --results kde-results.txt --output kde-certification.json
```

That record is distinct from the CLI preflight: it includes the exact session
metadata and one result for each required scenario, but remains
`certified: false` unless every scenario is explicitly passed, the doctor
snapshot reports `healthy: true`, and the selected backend probe is consistent
with the evidence metadata. Daemon-backed paths additionally require a status
snapshot with `response: "running"` and the matching source/backend pair; IBus
uses the doctor IBus-installation probe because it is not the daemon control
socket path. Its stable `status` field is `certified`, `incomplete`, or
`failed`.

JSON mode exits successfully when the report is produced; automation must
inspect `.certified`. Human-readable mode exits nonzero while certification is
incomplete.

The intended compositor harness will exercise the same checks against real
clients:

The checked-in target and scenario contract is
[`tests/certification/compositor-matrix.json`](../tests/certification/compositor-matrix.json).
CI validates that all four required desktop targets and all twelve scenarios
remain present, and rejects evidence that pairs a compositor with a backend
outside its declared certification paths.

| Environment | Required coverage |
| --- | --- |
| KDE Plasma / KWin | IBus, libei portal, input-method-v2 when exposed, Qt and GTK clients |
| GNOME | IBus, libei portal, input-method-v2 when exposed, GTK and Qt clients |
| Sway | evdev plus wlroots virtual keyboard, GTK and Qt clients |
| Hyprland | evdev plus wlroots virtual keyboard, GTK and Qt clients |

Each environment must cover printable press/release, auto-repeat, modifiers,
Unicode and combining text, multiline replacement, password fields, focus
transitions, cross-window isolation, configuration reload, daemon restart,
compositor restart, and failed insertion. Unsupported capabilities remain
explicit in the report; the harness must not convert an untested feature into
a pass.

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
a substitute for a real compositor session.
