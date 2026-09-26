# Historical Desktop Compositor Status

> **Historical snapshot — not current support evidence.** This page predates
> the certification matrix and contains exploratory or unverified claims. Do
> not use its “Verified” labels, test counts, or feature summaries for a
> deployment decision. The authoritative current status is
> [`docs/COMPOSITOR_MATRIX.md`](../COMPOSITOR_MATRIX.md), and a live session
> must produce explicit evidence through
> [`docs/CERTIFICATION.md`](../CERTIFICATION.md).

The material below is retained to document earlier implementation work and
must be read as historical context only.

## Summary

| Compositor | Verified | Capture | Output | Window Tracking | Notes |
|---|---|---|---|---|---|
| **KDE Plasma (KWin 6.6+)** | ✅ Yes | input-method-v2, evdev | virtual-keyboard | KWin D-Bus | Best-verified path; all features work |
| **Sway / wlroots** | 🔄 In Progress | input-method-v2, evdev | libei, virtual-keyboard | wlr-foreign-toplevel-v1 (Phase 2 ✅) | Foundation complete; needs daemon integration |
| **Hyprland / wlroots** | 🔄 In Progress | input-method-v2, evdev | libei, virtual-keyboard | wlr-foreign-toplevel-v1 (Phase 2 ✅) | Identical architecture to Sway |
| **river / wlroots** | ⚠️ Untested | input-method-v2, evdev | libei, virtual-keyboard | wlr-foreign-toplevel-v1 (Phase 2 ✅) | No real-world reports yet |
| **GNOME Shell** | ❌ Limited | input-method-v2 | —  | None | No window tracking, no libei; input-method-v2 only |

## Detailed Status by Compositor

### KDE Plasma Wayland (KWin 6.6+) ✅ **Verified**

**What works:**
- ✅ Text capture via `input-method-v2`
- ✅ Text injection via `wlroots virtual-keyboard`
- ✅ Password field detection (sensitive focus)
- ✅ Window tracking via KWin D-Bus (for `app_filter`)
- ✅ Fallback to evdev with limitations (see below)

**How to use:**
```bash
wayexpand daemon --source=input-method-v2
```

**Known limitations:**
- Escape, arrow keys, and F-keys may not pass through input-method-v2 (use evdev fallback)
- evdev capture (fallback) requires `input` group membership and has no password-field signal

**Evidence:**
- Live tested on KWin 6.6.6
- 170+ regression tests covering all core paths

---

### Sway / Hyprland / river (wlroots) 🔄 **In Progress**

**Current status:**
- ✅ Backend crates exist (evdev, libei, virtual-keyboard, input-method-v2)
- ✅ Protocol bindings vendored and buildable
- ✅ Window tracking infrastructure (Phase 1-2 complete, Phase 3 daemon integration pending)
- ⏳ **NOT YET INTEGRATED** into daemon event loop
- ❌ No real-world testing or reports

**What will work (once Phase 3 done):**
- Text capture via `input-method-v2` or evdev
- Text injection via libei or virtual-keyboard
- Window tracking via `wlr-foreign-toplevel-management-v1` (for `app_filter`)

**Expected timeline:**
- Phase 3 (daemon integration): ~1 hour
- Real-world testing: feedback-driven

**How to help:**
- Run `wayexpand doctor` on Sway/Hyprland and report output
- Test with `--source=input-method-v2` once Phase 3 lands
- Report what breaks on your specific version

---

### GNOME Shell Wayland ❌ **No Window Tracking**

**What works:**
- ✅ Text capture via `input-method-v2`
- ⚠️ Text injection: unclear (not libei portal, not easy virtual-keyboard)

**What doesn't work:**
- ❌ Window tracking (no standard protocol exists)
- ❌ Password field detection (input-method-v2 has no sensitive-focus event)
- ❌ `app_filter` (snippets restricted by app fail closed)

**Why:**
GNOME does not implement `wlr-foreign-toplevel-management` or expose focused window info to untrusted clients. This is a compositor design choice, not a WayExpand limitation.

**Workaround:**
- Use global hotkeys instead of `app_filter`
- Disable password-field expansion if you need it
- See [GNOME_WINDOW_TRACKING.md](GNOME_WINDOW_TRACKING.md) for details

**For maintainers / packagers:**
Document that WayExpand has limited GNOME support. Recommend to your users:
1. Use hotkeys instead of `app_filter`
2. Test with `wayexpand doctor` before relying on capture
3. Consider Sway/Hyprland if you need window-aware expansion

---

## Testing Checklist

Before claiming "Supported" status, a compositor must pass:

- [ ] Normal typing (ASCII)
- [ ] Unicode and emoji
- [ ] Deletion and backspace
- [ ] Text selection (delete selected text)
- [ ] Password field protection (if supported by backend)
- [ ] Application shortcuts (hotkeys work)
- [ ] Window focus changes (tracked correctly)
- [ ] Daemon reload (config changes apply)
- [ ] Multi-window scenarios (triggers don't cross boundaries)

## How to Contribute

1. **You're on Sway/Hyprland?**
   - Run `wayexpand doctor` and share output
   - Try the daemon once Phase 3 lands
   - Report what works and what breaks

2. **You're a package maintainer?**
   - Test on your target compositor
   - Report compatibility in packaging docs
   - Link to this page

3. **You want to implement support?**
   - See [PROFESSIONAL_ROADMAP.md](../../PROFESSIONAL_ROADMAP.md) Phase 3 for daemon integration
   - See [INTEGRATION_TESTING.md](../INTEGRATION_TESTING.md) for test setup

## Version Reference

- **Core engine & config:** Stable (v1.0.0+)
- **KDE Plasma support:** Verified (v1.0.0+)
- **wlroots backends:** Phase 1-2 in progress (Phase 3 ~ v1.2)
- **GNOME:** No window tracking (documented limitation)

Last updated: 2026-09-19
