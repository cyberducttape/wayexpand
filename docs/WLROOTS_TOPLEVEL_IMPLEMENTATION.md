# Wlroots Toplevel Management Implementation Guide

**Status:** In progress (architecture & scaffolding complete; protocol implementation pending)

**Compositors supported:** Sway, Hyprland, river, and other wlroots-based compositors

**Protocol:** `wlr_foreign_toplevel_management_unstable_v1`
- Spec: https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1
- Provides: Focused window identity (app_id) without D-Bus or scripting

---

## Overview

The wlroots toplevel tracker enables `app_filter`-scoped expansions on wlroots compositors by implementing the `wlr_foreign_toplevel_management_unstable_v1` Wayland protocol. This complements the existing KWin (KDE Plasma) backend and provides similar functionality for Sway, Hyprland, river, and other wlroots-based desktops.

### Key Differences from KWin Backend

| Aspect | KWin (D-Bus) | Wlroots (Wayland Protocol) |
|--------|---------|-------|
| **Transport** | D-Bus (KWin scripting API) | Wayland protocol globals |
| **Compositors** | KDE Plasma only | Sway, Hyprland, river, etc. |
| **Dependencies** | zbus (D-Bus client) | wayland-client, wayland-protocols |
| **Async Model** | Thread-based polling | Wayland event queue |
| **App ID source** | KWin window class | Wayland app_id property |

---

## Implementation Roadmap

### Phase 1: Basic Protocol Connection (Current)
- [x] Crate scaffolding: `crates/backend-wlroots-toplevel/`
- [x] Cargo.toml with wayland dependencies
- [x] Placeholder WindowTracker implementation
- [ ] Wayland display and registry connection
- [ ] Bind to `wlr_foreign_toplevel_manager_v1` global

**Effort:** 1 day
**Files:** `src/lib.rs` main implementation

### Phase 2: Event Handling
- [ ] Implement toplevel tracking via events:
  - `wlr_foreign_toplevel_v1::send_app_id` → extract app_id
  - `wlr_foreign_toplevel_v1::send_activated` → detect focus changes
- [ ] Build focused window cache
- [ ] Handle toplevel destruction/creation

**Effort:** 2-3 days
**Files:** `src/lib.rs` event loop, event types

### Phase 3: Timeout & Robustness
- [ ] Implement `next_window_timeout()` with proper blocking
- [ ] Handle unavailable compositor (graceful error)
- [ ] Cleanup on drop
- [ ] Test timeout edge cases

**Effort:** 1 day
**Files:** `src/lib.rs` error handling

### Phase 4: Testing & Integration
- [ ] Unit tests with mock Wayland server
- [ ] Integration tests on Sway/Hyprland/river
- [ ] Update SUPPORT_MATRIX.md
- [ ] Document troubleshooting

**Effort:** 2-3 days
**Files:** `src/lib.rs` tests, `docs/SUPPORT_MATRIX.md`

---

## Technical Design

### Wayland Protocol Flow

```
┌─ Wayland Client (WayExpand) ──────┐
│                                   │
│  1. Get wl_registry              │
│  2. Bind to                       │
│     wlr_foreign_toplevel_manager_v1
│  3. Listen for:                   │
│     - toplevel events             │
│     - app_id properties           │
│     - activate/deactivate signals │
│  4. Poll event queue              │
│     (non-blocking with timeout)   │
└───────────────────────────────────┘
```

### Data Flow

```rust
WindowContext {
    app_id: Some("org.swaywm.Alacritty"),  // from send_app_id event
    title: Some("zsh - Alacritty"),         // from send_title if available
}
```

---

## Code Structure

```
crates/backend-wlroots-toplevel/src/
├── lib.rs (300-400 lines)
│   ├── WlrootsTopLevelTracker struct
│   ├── WindowTracker trait impl
│   ├── Wayland client connection
│   ├── Event queue polling
│   └── Tests
```

### Key Components

**WlrootsTopLevelTracker:**
```rust
pub struct WlrootsTopLevelTracker {
    connection: wl_display,           // Wayland connection
    toplevel_manager: wlr_manager,    // Manager object
    toplevels: Vec<Toplevel>,         // Tracked windows
    focused_app_id: Option<String>,   // Current focused app
}
```

**Event Handlers:**
- `on_toplevel_app_id(app_id: String)` → update toplevels
- `on_toplevel_activated(index: usize)` → set focused
- `on_toplevel_closed(index: usize)` → cleanup

---

## Dependencies

**Already in workspace:**
- `wayland-client` (0.31)
- `wayland-protocols` (0.32)
- `wayland-protocols-misc` (0.3)
- `wayexpand-core` (local)

**New (minimal):**
- `thiserror` (already used elsewhere)

---

## Testing Strategy

### Unit Tests
- Mock Wayland server with dummy globals
- Simulate app_id events
- Verify WindowContext construction

### Integration Tests
- **Sway:** Test on actual Sway session (if available)
- **Hyprland:** Test on Hyprland session
- **Error cases:** Compositor without protocol support

### CI Integration
- Add to `.github/workflows/ci.yml` (runs on ubuntu-latest with Wayland)
- May need conditional skip if no Wayland in CI environment

---

## Known Limitations

1. **Wayland-only:** No support for X11 or Xwayland
2. **App ID reliability:** Depends on compositor implementation (usually reliable)
3. **Title support:** May not be available from all compositors
4. **Toplevel window only:** Does not track child windows

---

## Migration Path for Existing Users

**Current state (v1.1.2):**
- KDE Plasma: Works (KWin backend)
- GNOME/Mutter: No app-filter support
- Sway/Hyprland: No app-filter support

**After this implementation (v1.2):**
- KDE Plasma: Works (unchanged, KWin backend)
- Sway/Hyprland/river: Works (new wlroots-toplevel backend)
- GNOME/Mutter: No app-filter support (requires different protocol)

**For users:** No migration needed; daemon auto-selects appropriate backend at startup.

---

## Related Issues

- **P0 #20:** App-filter preview context (separate, complementary)
- **SUPPORT_MATRIX.md:** Will move wlroots from "not implemented" to "supported"
- **BACKENDS.md:** Will document new backend and limitations

---

## Next Steps

1. Implement Wayland display connection and registry binding
2. Add toplevel event handlers
3. Test on Sway/Hyprland
4. Update SUPPORT_MATRIX.md and documentation
5. Create integration tests
