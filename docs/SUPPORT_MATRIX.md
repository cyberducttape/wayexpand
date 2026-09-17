# Support matrix

This matrix separates implemented code from verified desktop behavior. A
Wayland session alone does not imply that a backend is usable.

| Area | Current status | Evidence required for promotion |
| --- | --- | --- |
| Core matching and config validation | Supported | Workspace unit tests and Clippy |
| CLI, JSON diagnostics, and config tooling | Supported | CLI and daemon smoke tests |
| systemd user services | Supported | `systemd-analyze verify` and installer test |
| wlroots virtual-keyboard output | Experimental | Target-compositor insertion test |
| libei/EIS output | Experimental | Portal consent, revocation, and reconnect tests; on-device tests for the `ei_keyboard`-only keysym-synthesis fallback (no `ei_text`), including non-US layouts |
| input-method-v2 capture | Experimental | Activation, focus, and Unicode tests |
| evdev capture (`--source=evdev`) | Experimental | Compositor-agnostic fallback for compositors without input-method-v2/virtual-keyboard support (e.g. KWin); has no sensitive-field signal (see `docs/SECURITY.md`) and does not yet forward key auto-repeat |
| Global hotkeys | Experimental | Backend capture and action tests |
| Focused-window tracking (`app_filter`) | Experimental (KDE Plasma only) | Live-verified against a real KWin 6.6 session in this repo's history; wlroots (`wlr-foreign-toplevel-management-unstable-v1`) and GNOME are not implemented |
| Key pass-through | Not supported | Must prove unrelated keys are never lost |
| Preedit/IME composition | Not supported | Native toolkit and compose/dead-key tests |

## Desktop coverage

No compositor is currently certified by automated end-to-end tests. Before
deploying desktop capture, test and record the exact compositor and version:

- Sway / wlroots
- Hyprland / wlroots
- KDE Plasma Wayland
- GNOME Shell Wayland

The integration procedure is in
[`docs/INTEGRATION_TESTING.md`](INTEGRATION_TESTING.md). A passing unit test,
backend probe, or running systemd process is not certification evidence.

## Promotion policy

A backend is promoted to Supported only after reproducible tests cover normal
typing, Unicode, deletion, selections, password fields, application
shortcuts, focus changes, compositor restart, and configuration reload. Known
limitations must be visible in `wayexpand doctor`, the GUI, and the release
notes.
