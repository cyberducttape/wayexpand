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

JSON mode exits successfully when the report is produced; automation must
inspect `.certified`. Human-readable mode exits nonzero while certification is
incomplete.

The intended compositor harness will exercise the same checks against real
clients:

| Environment | Required coverage |
| --- | --- |
| KDE Plasma / KWin | IBus, libei portal, Qt and GTK clients |
| GNOME | IBus, libei portal, GTK and Qt clients |
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
  then the safest verified local alternative.
- **Maximum compatibility** uses evdev with a detected libei/EIS path (or a
  KDE/GNOME portal candidate after explicit authorization) and warns that
  password-field awareness is unavailable.
- **Experimental** opts into input-method-v2 and clearly identifies its
  unsupported non-text-key and IME/preedit behavior.

Experts can inspect the underlying protocol decision with
`wayexpand explain-backend`. Administrators may continue to select exact
daemon source/backend arguments in service configuration.
