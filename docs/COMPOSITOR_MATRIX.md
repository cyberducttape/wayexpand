# Compositor Support Matrix

This is the authoritative support status. “Implemented” means code exists;
“certified” requires repeatable testing on a current compositor release and
common GTK, Qt, browser, terminal, and password-field targets.

| Session | Conservative daemon fallback | Available paths | Window tracking | Status |
|---|---|---|---|---|
| KDE Plasma / KWin | stdin test harness + libei; evdev only with explicit opt-in | IBus, input-method-v2, or evdev + libei when independently verified | KWin tracker | Implemented, awaiting broader independent certification |
| Sway | stdin test harness + libei; evdev only with explicit opt-in | evdev + wlroots virtual keyboard | Not shipped; wlroots tracking is experimental future work | Implemented, awaiting compositor certification |
| Hyprland | stdin test harness + libei; evdev only with explicit opt-in | evdev + wlroots virtual keyboard | Not shipped; wlroots tracking is experimental future work | Implemented, awaiting compositor certification |
| GNOME | stdin test harness + libei; evdev only with explicit opt-in | IBus, input-method-v2, or evdev + libei when independently verified | No supported app tracker | Limited; runtime probing required |
| X11 / XWayland | explicit evdev route | evdev + compatible output backend | No native tracker | Capture/output compatibility must be verified |

The conservative daemon fallback is deliberately limited: it never enables
raw evdev capture merely because `/dev/input` is readable. Explicit
`--source=evdev` acknowledges that global keyboard visibility. A successful
`doctor` probe is required before describing input-method-v2 as available on a
particular compositor. Wlroots window tracking is not currently shipped or
probeable. The `stdin` entry in the table is a test harness, not an ordinary
desktop capture service. `wayexpand setup` is the separate onboarding layer:
when the IBus
component is installed and permitted by organization policy, it recommends
IBus before these daemon fallback paths; the daemon selection column does not
claim that setup has already selected or enabled a service.

Certification evidence is collected with
[`scripts/certify-compositor.sh`](../scripts/certify-compositor.sh). It records
live probes and requires explicit results for printable press/release, held
keys and repeat, modifiers/navigation, Unicode, multiline/rapid typing,
password fields, focus/cross-window isolation, reload, daemon/compositor
restart, failed insertion, and IME/preedit behavior; an unmarked scenario is
never treated as certified.
