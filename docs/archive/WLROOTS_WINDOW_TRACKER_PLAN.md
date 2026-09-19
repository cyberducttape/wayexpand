# wlroots Window Tracker Implementation Plan (1.1+)

**Status:** Planning phase, deferred to 1.1 after 1.0 release  
**Compositors:** Sway, Hyprland, and other wlroots-based compositors  
**Protocol:** `wlr-foreign-toplevel-management-unstable-v1`

---

## Overview

The wlroots window tracker enables app-scoped expansions (`app_filter`) on Sway,
Hyprland, and other wlroots-based compositors. Currently, only KDE Plasma (KWin)
is supported via the D-Bus scripting interface.

**Current State:**
- `crates/backend-kwin-window/` — KWin-specific D-Bus implementation (1.0)
- `crates/backend-wlroots-window/` — Does not exist yet
- Protocol library available: `wayland-protocols-wlr` in Cargo.lock (vendored)

**Why deferred to 1.1:**
1. Requires testing on actual Sway/Hyprland sessions (§5.1 in checklist)
2. No time in 1.0 schedule for unfamiliar protocol implementation
3. Standard protocol (`wlr-foreign-toplevel-management-unstable-v1`) is lower
   risk than KWin's D-Bus bridge, so safer for 1.1
4. Users can use global expansions (no `app_filter`) as workaround

---

## Architecture

### Protocol Overview

`wlr-foreign-toplevel-management-unstable-v1` provides:
- **Manager:** `zwlr_foreign_toplevel_manager_v1` (get from registry)
- **Toplevel:** `zwlr_foreign_toplevel_v1` (one per window)
- **Events:**
  - `output` — window appears on output
  - `activated` — window focus changed
  - `done` — batch end marker
  - `closed` — window destroyed

### Proposed Crate Structure

```
crates/backend-wlroots-window/
├── src/
│   ├── lib.rs              — public WindowTracker impl
│   ├── toplevel.rs         — window tracking state machine
│   └── protocol.rs         — wayland protocol bindings
├── Cargo.toml
└── tests/
    └── integration.rs      — smoke test (requires Sway/Hyprland)
```

---

## Implementation Phases

### Phase 1: Scaffold (1-2 days)

**Goal:** Project structure + basic protocol connection

**Tasks:**
1. Create `crates/backend-wlroots-window/` crate
2. Add `wayland-client` dependency (Wayland protocol library)
3. Implement protocol discovery:
   ```rust
   pub fn probe() -> Result<(), WlrootsWindowError> {
       let conn = wayland_client::Connection::connect_to_env()?;
       let registry = conn.display().get_registry();
       // Check: does registry have zwlr_foreign_toplevel_manager?
   }
   ```
4. Implement connection (similar to KWin structure):
   ```rust
   pub struct WlrootsWindowTracker {
       manager: zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
       current_window: Mutex<Option<WindowContext>>,
       toplevels: Mutex<Vec<Toplevel>>,
   }
   ```
5. Add to daemon `--backend=wlroots-window` option (stub)

**Deliverable:**
- Compiles but functions not yet implemented
- Can probe for protocol availability
- Test: `wayexpand doctor` reports "wlroots-window: available/unavailable"

**Estimated effort:** 4-6 hours

### Phase 2: Event Handling (2-3 days)

**Goal:** Track window focus changes

**Tasks:**
1. Implement event loop:
   ```rust
   impl WindowTracker for WlrootsWindowTracker {
       fn next_window_timeout(&mut self, timeout: Duration) 
           -> Result<Option<Option<WindowContext>>, WindowTrackerError>
       {
           // Event loop: wait for activated event
           // Extract: app_id + title from toplevel properties
           // Return: WindowContext or None (desktop focus)
       }
   }
   ```
2. Map toplevel properties to `WindowContext`:
   - `app_id` → `window.app_id`
   - `title` → `window.title`
   - Handle desktop focus (empty app_id + title = `None`)

3. Handle edge cases:
   - Multiple outputs (multi-monitor)
   - Window closed while tracking
   - Compositor restart during tracking

**Deliverable:**
- `next_window_timeout()` returns focused window or desktop
- Smoke test: manual focus change, verify returned app_id

**Estimated effort:** 6-8 hours

### Phase 3: Testing & Hardening (1-2 days)

**Goal:** Reliability and compositor compatibility

**Tasks:**
1. Stress test:
   - Rapid focus changes
   - Multiple windows with same app_id
   - Window creation/destruction during read
   - Output hotplug

2. Compatibility matrix:
   - Test on Sway (latest stable)
   - Test on Hyprland (latest stable)
   - Document version requirements

3. Error handling:
   - Protocol version mismatch (unstable-v1 may change)
   - Manager unavailable
   - Timeout during event wait

4. Regression test:
   - Verify KWin behavior unchanged (if both compiled)
   - Verify fallback when unavailable

**Deliverable:**
- Integration tests in `tests/integration.rs` (runs on Sway/Hyprland)
- Compatibility doc in `docs/`

**Estimated effort:** 4-6 hours

---

## Code Comparison: KWin vs wlroots

### KWin (Current Implementation)

```
+---------+        D-Bus         +----------+
| WayExpand|<--session bus---->| KWin     |
|  daemon |                     | Scripting|
+---------+                     +----------+
   |                                  |
   | (scripting API)                  |
   +----------> [KWin script]
             (JavaScript in /tmp)
                  |
              watches workspace.windowActivated
              calls back via D-Bus service
```

**Advantages:**
- No Wayland protocol implementation needed
- Works in any KDE environment (via D-Bus)

**Disadvantages:**
- KDE-specific
- D-Bus overhead
- Script registration race condition
- JavaScript in /tmp (cleanup concern)

### wlroots (Proposed for 1.1)

```
+---------+       Wayland        +----------+
| WayExpand|<--UNIX socket---->| Sway/     |
|  daemon  |  foreign_toplevel  | Hyprland |
+---------+                     +----------+
   |
   | (standard protocol, no D-Bus)
   +-> Wayland event loop
       listens for activated events
       (synchronous, no script needed)
```

**Advantages:**
- Standard protocol (lower maintenance)
- Works on all wlroots compositors
- No runtime script injection
- No race conditions
- Simpler error handling

**Disadvantages:**
- Wayland protocol implementation (more code)
- Requires `wayland-client` dependency
- Compositor must be fully loaded (not a blocker)

---

## Dependencies

**To add for wlroots tracker:**
```toml
[dependencies]
wayland-client = "0.31"  # Wayland protocol client library
wayland-protocols = "0.31"  # Standard protocols
wayland-protocols-wlr = "0.31"  # wlroots extensions (already in Cargo.lock)
```

**Note:** `wayland-protocols-wlr` is already vendored in the workspace;
`wayland-client` and `wayland-protocols` are lightweight and well-maintained.

---

## Testing Strategy

### Unit Tests (CI)
- Protocol discovery logic
- Event parsing
- Edge case handling (None contexts, malformed events)

### Integration Tests (Requires Real Compositor)
- Run on Sway/Hyprland in CI environment (if available)
- Fallback: Manual testing on target compositors before release

### Compatibility Matrix

| Compositor | Version | Status | Blocker? |
|------------|---------|--------|----------|
| Sway       | 1.8+    | TBD    | No       |
| Hyprland   | 0.40+   | TBD    | No       |
| Niri       | 0.1+    | Maybe  | No       |
| Wayfire    | 0.8+    | Maybe  | No       |

---

## Success Criteria for 1.1

**Minimum viable product:**
- [ ] `WlrootsWindowTracker::new()` creates protocol connection
- [ ] `next_window_timeout()` returns focused app_id + title
- [ ] Handles desktop focus (returns `None`)
- [ ] Handles window close mid-tracking
- [ ] Tested on Sway + Hyprland
- [ ] Documented in SUPPORT_MATRIX.md as "Supported"

**Nice-to-have:**
- [ ] Handle multiple outputs correctly
- [ ] Pre-cache toplevel list for faster initial read
- [ ] Support for future unstable-v2 (if released)

---

## Known Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Protocol changes (`unstable-v1` can evolve) | Monitor wayland-protocols updates; version-pin in Cargo.lock |
| Compositor doesn't support protocol | Probe + graceful fallback (already done in KWin code) |
| Wayland-client library complexity | Well-maintained by Smithay; minimal custom protocol code |
| Testing on CI (need Sway/Hyprland) | Manual testing before release; optional CI step |
| Regression in KWin support | Keep `crates/backend-kwin-window/` unchanged; test both |

---

## Timeline for 1.1

**Estimate:**
- Phase 1 (Scaffold): 4-6 hours (start of 1.1 cycle)
- Phase 2 (Events): 6-8 hours (mid-cycle)
- Phase 3 (Testing): 4-6 hours (pre-release)
- **Total:** 14-20 hours of development time

**Recommended 1.1 schedule:**
- Months 1-2: Phase 1 + Phase 2
- Month 2: Phase 3 (testing)
- Month 3: 1.1 release

---

## Backward Compatibility

1.0 ships with `app_filter` "KDE Plasma only" (documented).  
1.1 upgrades to "KDE Plasma + wlroots compositors" (backward-compatible).  
Users with no `app_filter` config are unaffected.

---

## Future Enhancements (Post-1.1)

- [ ] GNOME Shell support (requires shell extension, lower priority)
- [ ] X11 support (deprecated, lower priority)
- [ ] Cached app_id for instant re-entry ("last app" button)
- [ ] Per-app hotkeys (using window tracker)
- [ ] Notification on app change (debug mode)

---

## See Also

- [`RELEASE_1.0_CHECKLIST.md` §5.1](RELEASE_1.0_CHECKLIST.md#5-this-sessions-own-follow-through-items) — Original deferred item
- [`SUPPORT_MATRIX.md`](../SUPPORT_MATRIX.md) — Compositors requiring testing
- [`crates/backend-kwin-window/`](../../crates/backend-kwin-window/) — Reference implementation (KWin)
- [`wayland-protocols-wlr`](../../Cargo.lock) — Protocol definitions (already vendored)
