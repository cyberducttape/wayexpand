# Compositor Support Matrix

This is the authoritative support status. “Implemented” means code exists;
“certified” requires repeatable testing on a current compositor release and
common GTK, Qt, browser, terminal, and password-field targets.

| Session | Automatic choice | Available paths | Window tracking | Status |
|---|---|---|---|---|
| KDE Plasma / KWin | stdin + libei; evdev only with explicit opt-in | input-method-v2 only when `doctor` confirms it; evdev + libei | KWin tracker | Implemented, awaiting broader independent certification |
| Sway | stdin + libei; evdev only with explicit opt-in | evdev + libei; input-method-v2 only when probed | Not shipped; wlroots tracking is experimental future work | Implemented, awaiting compositor certification |
| Hyprland | stdin + libei; evdev only with explicit opt-in | evdev + libei; input-method-v2 only when probed | Not shipped; wlroots tracking is experimental future work | Implemented, awaiting compositor certification |
| river | stdin + libei; evdev only with explicit opt-in | evdev + libei; input-method-v2 only when probed | Not shipped; wlroots tracking is experimental future work | Implemented, awaiting compositor certification |
| GNOME | stdin + libei; evdev only with explicit opt-in | input-method-v2 (explicit opt-in) or evdev + libei | No supported app tracker | Limited; runtime probing required |
| X11 / XWayland | explicit evdev route | evdev + compatible output backend | No native tracker | Capture/output compatibility must be verified |

The daemon’s automatic choice is deliberately conservative: it never enables
raw evdev capture merely because `/dev/input` is readable. Explicit
`--source=evdev` acknowledges that global keyboard visibility. A successful
`doctor` probe is required before describing input-method-v2 as available on a
particular compositor. Wlroots window tracking is not currently shipped or
probeable.
