# GUI Performance Analysis & Optimization Roadmap

## Overview

This document analyzes known performance bottlenecks in the WayExpand GUI and proposes improvements for post-1.0 releases.

---

## Known Issues

### "Use current app" Button Freezes UI (1-2 seconds)

**Symptom:**
When clicking the "🎯 Use current app" button in the Snippet Editor, the GUI becomes unresponsive for 1-2 seconds before displaying the result.

**Root Cause:**
The button handler runs synchronously on the UI thread (line 1269-1315 in `crates/gui/src/main.rs`):

```rust
if detect_app_clicked {
    let detection = match KwinWindowTracker::new() {
        Ok(mut tracker) => {
            match tracker.next_window_timeout(std::time::Duration::from_secs(5)) {
                // ... handle result
            }
        }
    }
}
```

**Timeline of Blocking Operations:**
1. **D-Bus connection setup** (~10-50ms)
2. **KWin service registration** (~100-200ms)
3. **KWin script loading + registration race** (up to 2.25s)
   - `loadScript()` returns immediately
   - `/Scripting/ScriptN` D-Bus object takes time to register
   - `run()` call retried up to 15 times with 150ms delays
4. **Wait for first window callback** (< 500ms on focused window)
5. **String formatting + UI update** (< 10ms)

**Total observed:** 1-2 seconds (mostly KWin script registration delay)

**Severity:** Low — User experience papercut, not a correctness issue. Affects only:
- First app filter creation
- Switching between snippets with/without app filters
- KWin session where script loading is slow

---

## Proposed Solutions

### Short-term (1.0 Maintenance)

**Document the behavior:** ✅ DONE — Added to [Troubleshooting.md](docs/wiki/Troubleshooting.md)
- Explains expected 1-2 second delay
- Clarifies it only happens on first enable
- Points to retry constants for advanced users

### Medium-term (1.1+)

#### Option A: Background Thread with Spinner (Recommended)

Move tracker initialization to a background thread and show a spinner while waiting.

**Implementation:**
```rust
// In egui message-passing loop:
enum Message {
    DetectAppClicked,
    AppDetectionComplete(Result<WindowContext>),  // from background thread
}

// On button click:
send_message(Message::DetectAppClicked);  // spawns thread

// In update handler:
Message::AppDetectionComplete(result) => {
    // Update draft.app_filter
}
```

**Benefits:**
- UI remains responsive
- Visual feedback via spinner
- Timeout still bounds the operation (5 seconds)

**Trade-off:**
- More complex code (message-passing, threading)
- Requires egui integration with async/threading

**Estimated effort:** 4-6 hours

#### Option B: Pre-load Tracker on Startup

Initialize KWin tracker once during GUI startup (if available) and keep it alive.

**Implementation:**
```rust
struct State {
    kwin_tracker: Option<KwinWindowTracker>,  // Loaded once at startup
}
```

**Benefits:**
- First button click is instant (tracker already loaded)
- Single script + D-Bus service for entire GUI lifetime

**Trade-off:**
- Uses D-Bus connection + KWin script even if never used
- Cleanup on GUI exit required
- If KWin restarts, tracker becomes stale

**Estimated effort:** 2-4 hours

#### Option C: Async/await with Tokio

Refactor GUI event loop to support async operations.

**Benefits:**
- Modern Rust async patterns
- Easier to add other async operations later

**Trade-off:**
- Large refactor (egui doesn't natively support async)
- Requires runtime (tokio) in GUI process
- Complex integration

**Estimated effort:** 12-20 hours

---

## Recommendation for 1.1

**Primary:** Option A (Background Thread + Spinner)
- Best balance of UX improvement (1-2s freeze → instant UI + spinner)
- Moderate implementation complexity
- Follows UI best practices

**Fallback:** Option B (Pre-load)
- Simpler if Option A proves complex
- Nearly instant after startup cost

**Defer:** Option C (Full Async)
- Useful for 2.0+ when other async needs accumulate
- Not worth the refactor for a single operation

---

## Testing Strategy

### Current Behavior (Baseline)
1. Click "Use current app" button
2. Measure GUI freeze time with `systemd-analyze` or frame profiler
3. Expected: 1-2 seconds

### After Optimization
1. Click button
2. Spinner displays immediately
3. Measure spinner duration
4. Expected: < 200ms until result displayed (spinner shows work in progress)

### Edge Cases to Test
- [ ] Button clicked multiple times rapidly
- [ ] KWin restarts while GUI is open
- [ ] KWin version mismatch (6.0, 6.6, 7.0)
- [ ] D-Bus connection temporarily unavailable
- [ ] GUI window loses focus while detection pending

---

## Performance Metrics to Track

For future profiling:
```
- Time: KwinWindowTracker::new()  [D-Bus + script load]
  - Current: 1-2 seconds (KWin 6.6.6)
  - Varies: by KWin version (5.x vs 6.x)

- Time: next_window_timeout(5s)  [waiting for callback]
  - Current: < 500ms (focused window)
  - Varies: depends on when focus changes

- UI responsiveness (frame drops)
  - Current: detectable pause every frame during 1-2s wait
  - After optimization: 0 frame drops (background thread)
```

---

## Code Locations

- **Button handler:** `crates/gui/src/main.rs:1269-1315`
- **KWin tracker:** `crates/backend-kwin-window/src/lib.rs:101-129`
- **Retry logic:** `crates/backend-kwin-window/src/lib.rs:137-170` (constants at 24-25)
- **GUI framework:** egui (not async-native)

---

## Not Blocking 1.0

This is a known limitation that:
1. Only affects KDE Plasma (other compositors report "unavailable" instantly)
2. Is not a correctness issue (works as designed, just slow)
3. Affects an optional feature (app filters)
4. Is properly documented for users

**Decision:** Document and defer to 1.1

---

## Future Considerations

Once Option A or B is implemented in 1.1, consider:
- [ ] Profile other slow operations in GUI startup
- [ ] Pre-load other backend trackers if available
- [ ] Add configuration for "auto-detect app on startup" (pre-fill field)
- [ ] Cache last-detected app_id for faster re-entry
