# Compositor Support Matrix

This is the authoritative support status. “Implemented” means code exists;
“certified” requires repeatable testing on a current compositor release and
common GTK, Qt, browser, terminal, and password-field targets.

| Session | Automatic choice | Available paths | Window tracking | Status |
|---|---|---|---|---|
| KDE Plasma / KWin | evdev + libei | input-method-v2 only when `doctor` confirms it; evdev + libei | KWin tracker | Implemented, awaiting broader independent certification |
| Sway | evdev + libei | evdev + libei; input-method-v2 only when probed | wlroots scaffold, not active tracking | Implemented, awaiting compositor certification |
| Hyprland | evdev + libei | evdev + libei; input-method-v2 only when probed | wlroots scaffold, not active tracking | Implemented, awaiting compositor certification |
| river | evdev + libei | evdev + libei; input-method-v2 only when probed | wlroots scaffold, not active tracking | Implemented, awaiting compositor certification |
| GNOME | input-method-v2 only when `doctor` confirms it | input-method-v2 or evdev + libei | No supported app tracker | Limited; runtime probing required |
| X11 / XWayland | explicit evdev route | evdev + compatible output backend | No native tracker | Capture/output compatibility must be verified |

The daemon’s automatic choice is deliberately conservative. Explicit
`--source` and `--backend` options override it. A successful `doctor` probe is
required before describing input-method-v2 or wlroots window tracking as
available on a particular compositor.
