# Wlroots Window Tracking Implementation Guide

## Overview

Window tracking for wlroots-based compositors (Sway, Hyprland, river) using the `wlr-foreign-toplevel-management-unstable-v1` protocol. Currently only KDE Plasma (D-Bus) is implemented.

## Architecture

### Current State

- **KDE Plasma**: `backend-kwin-window` uses D-Bus org.kde.kwin.Scripting interface
- **Input-method-v2**: Provides keyboard focus but no window identity
- **evdev**: No window identity available

### Proposed wlroots Backend

New crate: `crates/backend-wlroots-toplevel` implementing `WindowTracker` trait

```
WindowTracker trait:
  ├── new(timeout: Duration) -> Result<Self>
  ├── current_window() -> Option<WindowContext>
  └── set_callback(Fn(&Option<WindowContext>))
```

## Implementation Phases

### Phase 1: Protocol Connection (Essential)

**File**: `crates/backend-wlroots-toplevel/src/lib.rs`

**Steps**:
1. Initialize Wayland connection using `wayland-client`
2. Bind to global `wlr_foreign_toplevel_manager_v1`
3. Handle toplevel creation/destruction events
4. Store active toplevel list with app_id and title

**Key APIs**:
```rust
use wayland_client::protocol::wl_registry;
use wayland_protocols_wlr::foreign_toplevel_management::v1::client::*;

// Bind to manager in registry
manager: zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1
// Listen to toplevel events
event: zwlr_foreign_toplevel_manager_v1::Event::Toplevel { .. }
```

**Tests**:
- Registry bind succeeds on Sway/Hyprland/river
- Toplevel list updates on window creation
- App_id and title correctly extracted

### Phase 2: Focus Tracking (Essential)

**File**: Extend `src/lib.rs`

**Steps**:
1. Listen to `focused` event from each toplevel
2. When focused-changed event arrives, extract app_id + title
3. Convert to `WindowContext` struct
4. Trigger callback if subscribed

**Event Flow**:
```
toplevel.focused() 
  -> zwlr_foreign_toplevel_handle_v1::Event::Focused(output)
  -> Extract app_id from toplevel.app_id property
  -> Extract title from toplevel.title property
  -> Create WindowContext { app_id, title }
  -> Call callback(&Some(window_context))
```

**Tests**:
- Focused event detected on window switch
- Correct app_id extracted (e.g., "firefox", "sway")
- Correct title extracted
- Callback fires with correct window

### Phase 3: Daemon Integration (Required for v1.2)

**File**: `crates/daemon/src/main.rs`

**Steps**:
1. Add to backend discovery: `discover_window_tracker()` checks if wlroots is available
2. Instantiate wlroots backend if requested or auto-detected
3. Connect callback to daemon's window-change event
4. Route to `engine.process(InputEvent::WindowChanged(window))`

**Discovery**:
```rust
pub fn discover_wlroots_toplevel() -> (BackendState, String) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return (BackendState::Unavailable, "WAYLAND_DISPLAY not set".into());
    }
    
    match WlrootsToplevelTracker::new(Duration::from_secs(5)) {
        Ok(_) => (BackendState::Implemented, "wlr-foreign-toplevel-management-v1 available".into()),
        Err(e) => (BackendState::Unavailable, format!("probe failed: {e}")),
    }
}
```

**Tests**:
- Backend discovered on Sway/Hyprland/river
- Correctly reports unavailable on GNOME/KDE
- Window context propagates to engine

### Phase 4: Integration Testing (Recommended for v1.2)

**File**: `crates/backend-wlroots-toplevel/src/lib.rs` (test module)

**Tests** (requires manual setup on each compositor):

1. **Sway Test Setup**:
```bash
sway --config /tmp/sway-test-config
# In sway, switch windows and verify app_id changes
```

2. **Hyprland Test Setup**:
```bash
Hyprland --config /tmp/hyprland-test-config
# Switch windows and verify tracking
```

3. **river Test Setup**:
```bash
river
# Switch windows and verify tracking
```

**Functional Tests**:
- [ ] App_id reported for native Wayland apps (Firefox, GNOME apps, etc.)
- [ ] App_id reported for XWayland apps (older GTK apps, etc.)
- [ ] Window title extracted correctly
- [ ] Focus changes detected within 100ms
- [ ] app_filter matching works (e.g., `"firefox"` matches Firefox window)

## Protocol Reference

### wlr-foreign-toplevel-management-v1

**Spec**: https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1

**Key Messages**:
- `get_toplevel()` - returns zwlr_foreign_toplevel_handle_v1
- `toplevel.app_id()` - UTF-8 app identifier
- `toplevel.title()` - UTF-8 window title
- `toplevel.focused(output)` - window gained focus
- `toplevel.done()` - batch end marker

**Lifecycle**:
```
manager.toplevel(handle)
  -> handle.app_id("firefox")
  -> handle.title("GitHub - New Issue")
  -> handle.focused()
  -> handle.done()
```

## Known Issues & Workarounds

1. **XWayland Window Titles**: XWayland apps (e.g., older GTK) may have generic titles or no app_id
   - Workaround: Fall back to app_id if title is generic
   - Tracking: See GNOME/GTK documentation

2. **Transient Windows**: Dialogs/popups may report parent's app_id
   - Solution: Ignore transient windows or use parent app_id

3. **Desktop Windows**: The desktop itself may report as a toplevel
   - Solution: Filter out common desktop app_ids (sway, wlr-panel, etc.)

## Dependencies

- `wayland-client`: Wayland protocol bindings
- `wayland-protocols-wlr`: wlroots protocol definitions (vendored)

Add to `crates/backend-wlroots-toplevel/Cargo.toml`:
```toml
wayland-client = { workspace = true }
wayland-protocols-wlr = { workspace = true, features = ["client"] }
```

## Verification Checklist

- [ ] Protocol bindings available in wayland-protocols-wlr crate
- [ ] Focus tracking works on at least one test compositor
- [ ] Window context correctly propagates to engine
- [ ] app_filter matching uses app_id (primary) or title (fallback)
- [ ] No memory leaks with repeated window switches
- [ ] Graceful degradation if protocol not available
- [ ] doctor command correctly reports availability
- [ ] SUPPORT_MATRIX.md updated

## Timeline Estimate

- **Phase 1 (Protocol)**: ~2 hours
- **Phase 2 (Focus tracking)**: ~2 hours  
- **Phase 3 (Daemon integration)**: ~1 hour
- **Phase 4 (Testing)**: ~2 hours per compositor

**Total**: ~10-12 hours for production-ready implementation

## References

- [wlr-foreign-toplevel-management-unstable-v1 spec](https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1)
- [Sway source code examples](https://github.com/swaywm/sway)
- [wayland-client documentation](https://docs.rs/wayland-client/)
- Related: `backend-kwin-window` for KDE Plasma reference implementation
