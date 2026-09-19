# Phase 3: Daemon Integration - Wlroots Window Tracking

> **Archived historical plan.** The prototype described here was removed from
> the production workspace because it was disabled and not ready for compositor
> use. Wlroots window tracking is not currently shipped; see
> [BACKENDS.md](../BACKENDS.md) for the current boundary.

This document describes Phase 3 of the wlroots window tracking implementation: integrating the tracker into the daemon event loop so `app_filter`-scoped snippets work on Sway, Hyprland, and river.

## What Phase 3 Accomplishes

**Phase 3 wires the window tracker into the daemon's main event loop:**
- Daemon detects available trackers (KWin first, then wlroots)
- Runs tracker in a background thread
- Sends `InputEvent::WindowChanged` to the engine when focus changes
- Engine uses focused window for `app_filter` matching

**Result:** Snippets with `app_filter = ["firefox"]` now work on wlroots compositors.

## Architecture

### Before Phase 3 (Incomplete)

```
Daemon Main Loop          WlrootsToplevelTracker (Phase 1-2)
    │                            │
    ├─ Input capture             ├─ Protocol connection
    ├─ Text expansion            ├─ Toplevel discovery
    └─ Text injection            └─ Focus tracking
                                     (events to channel, unused)
```

### After Phase 3 (Complete)

```
Daemon Main Loop          WlrootsToplevelTracker (Phase 1-2-3)
    │                            │
    ├─ Input capture             ├─ Protocol connection
    ├─ Text expansion ◄──────────┤─ Toplevel discovery
    │   (uses window context     ├─ Focus tracking
    │    for app_filter)         └─ Send WindowChanged to engine
    └─ Text injection
    
    Window changes → InputEvent::WindowChanged → app_filter matching
```

## Implementation Details

### Daemon Changes (crates/daemon/src/main.rs)

**1. Import wlroots backend:**
```rust
use wayexpand_backend_wlroots_toplevel::WlrootsToplevelTracker;
```

**2. Update spawn_window_tracker():**
```rust
fn spawn_window_tracker() -> Option<mpsc::Receiver<Option<WindowContext>>> {
    // Try KWin first (most reliable on KDE Plasma)
    if KwinWindowTracker::probe().is_ok() {
        // Launch KWin tracker thread
        return Some(receiver);
    }

    // Try wlroots (Sway, Hyprland, river, etc.)
    match WlrootsToplevelTracker::new(Duration::from_secs(3)) {
        Ok(_) => {
            // Launch wlroots tracker thread
            Some(receiver)
        }
        Err(_) => {
            // No tracker available
            None
        }
    }
}
```

### Event Flow

1. **Window focus changes** (user Alt-Tabs)
2. **wlr-foreign-toplevel protocol fires focused event**
3. **WlrootsToplevelTracker receives event and sends window via mpsc channel**
4. **Main event loop drain_pending_window_events()**
5. **engine.process(InputEvent::WindowChanged(window))**
6. **Engine updates current_window for app_filter matching**

## How app_filter Works with Window Tracking

**Configuration:**
```toml
[[expansion]]
trigger = ":ff"
replacement = "Firefox"
app_filter = ["firefox"]
```

**Behavior:**
- When focused window's app_id is "firefox", `:ff` → `Firefox`
- When focused window's app_id is "code", `:ff` doesn't match (fail closed)
- When window tracking unavailable, `:ff` doesn't match (fail closed)

## Testing Phase 3

### Manual Testing

**On Sway/Hyprland:**
```bash
# Start daemon
wayexpand daemon --source=input-method-v2

# In another terminal, check daemon is running
systemctl --user status wayexpand

# Check logs to verify tracker initialized
journalctl --user -u wayexpand -n 20 | grep "window tracker"

# Expect output like:
# INFO wayexpand_daemon: window tracker active (wlr-foreign-toplevel-management-v1)
```

**Create a test snippet:**
```toml
[[expansion]]
trigger = ":test"
replacement = "IT WORKS"
app_filter = ["code"]
```

**Test:**
1. Open VS Code (or firefox, or any app)
2. Focus the window
3. Type `:test` → should expand to `IT WORKS` only in VS Code
4. Switch to another app and type `:test` → should NOT expand

### Automated Testing

**Run the daemon tests:**
```bash
cargo test --locked -p wayexpand-daemon reload
```

**Run all tests:**
```bash
cargo test --locked --workspace
```

All 191+ tests should pass.

## Limitations and Known Issues

### Sway/Hyprland/river Specific

1. **Password field detection:** Not available via wlr-foreign-toplevel
   - Use `--source=input-method-v2` for password field detection
   - Fallback: `--source=evdev` (no password field detection, requires `input` group)

2. **Key pass-through:** Escape, arrows, F-keys may not work
   - input-method-v2 limitation
   - Use evdev if this is critical, with the security tradeoff

3. **Window tracking may lag:** Up to 2 seconds (polling interval)
   - Acceptable for most users (focus changes are usually intentional)
   - Can be tuned by changing timeout in spawn_window_tracker

### GNOME Shell

- No window tracking protocol exists
- app_filter snippets fail closed (never match)
- No support planned (GNOME design choice)

## Verification Checklist

Use this to verify Phase 3 is working correctly:

- [ ] Daemon builds without warnings
- [ ] `systemctl --user start wayexpand` succeeds
- [ ] `journalctl --user -u wayexpand` shows "window tracker active"
- [ ] `app_filter` snippets match only in correct windows
- [ ] Switching windows doesn't trigger snippets in wrong apps
- [ ] Config reload preserves window context (P0 fix ✅)
- [ ] `wayexpand doctor` reports window tracking as "Implemented"
- [ ] All 191 tests pass

## Debugging

**If window tracking doesn't work:**

1. **Check if tracker is running:**
   ```bash
   journalctl --user -u wayexpand -n 50 | grep -i "window tracker"
   ```

2. **Verify protocol availability:**
   ```bash
   wayexpand doctor
   ```
   Should show window tracking status.

3. **Check if you're on a supported compositor:**
   ```bash
   echo $XDG_CURRENT_DESKTOP
   # Should be "KDE" (KWin) or "wlroots" variant
   ```

4. **Try forcing wlroots:**
   ```bash
   WAYLAND_DISPLAY=wayland-1 wayexpand daemon --source=input-method-v2
   ```

## References

- [docs/WLROOTS_WINDOW_TRACKING_GUIDE.md](WLROOTS_WINDOW_TRACKING_GUIDE.md) - Protocol details
- [docs/DESKTOP_STATUS.md](DESKTOP_STATUS.md) - Per-compositor support
- Historical prototype implementation: removed; no production crate currently exists
- [crates/daemon/src/main.rs](../../crates/daemon/src/main.rs) - Integration point

## Timeline and Commits

- **Phase 1** (1f58bc43): Protocol scaffolding
- **Phase 2** (0f48b022): Event-driven focus tracking
- **Phase 3** (this): Daemon integration - window changes now flow to engine

Total time: ~4 hours across three phases. Next: real-world testing on Sway/Hyprland/river.
