# P0-3 Decision Required: Trailing-Character Deletion Race Condition

**Date:** 2026-09-19  
**Status:** Awaiting stakeholder decision  
**Impact:** Blocks final production-ready claim

## The Problem

With evdev non-exclusive capture, keystroke delivery is asynchronous to matcher processing. When a user types very quickly, the keystroke that terminates a word-boundary trigger can reach the application before the matcher has finished deciding whether to expand and delete.

### Example Scenario

1. User types rapidly: `"hello :sig "` (colon, s, i, g, space)
2. Matcher receives characters one at a time via evdev: `"h"`, `"e"`, `"l"`, `"l"`, `"o"`, `" "`, `":"`, `"s"`, `"i"`, `"g"`
3. At `"g"`, matcher recognizes `:sig` trigger and prepares deletion + expansion
4. Meanwhile, the `" "` keystroke event has already been delivered to the application
5. Current implementation: Uses `reinsert_after` field to delete both `:sig` AND the space, then insert replacement + space
6. **Race condition:** If app receives `" "` before matcher sends deletion, the result can be garbled

### Current Partial Fix

The `reinsert_after` field (implemented in v1.1.1+) handles the standard case by telling backends to erase both the trigger AND the terminating character, then re-insert the terminator after the replacement. This works for typical typing speed.

### When It Breaks

Very fast typists or systems with high input processing latency can hit the race. The space/Enter/other terminating character is already in the app when the matcher tries to delete it.

## Decision Options

### Option 1: Key-Event Queuing (Fix the Race Completely) ✅ Recommended for Correctness

**Approach:** Implement key-event batching in backends. Instead of immediately forwarding keystroke events to the app, queue them until the matcher completes its decision. This ensures deletion happens atomically with app delivery.

**Implementation:**
- Modify evdev backend to buffer key events
- Only forward to app after matcher has processed and made decision
- Requires synchronization between input thread and matcher
- Small latency increase (microseconds)

**Pros:**
- ✅ Eliminates race condition completely
- ✅ Guarantees correctness for all typing speeds
- ✅ No feature limitations
- ✅ Transparent to users

**Cons:**
- ⚠️ Complex implementation (~8-12 hours)
- ⚠️ Requires architecture change (thread synchronization)
- ⚠️ Slight latency increase (< 1ms in practice)

**Recommendation:** Choose this if correctness and full feature support are critical.

### Option 2: Document as Known Limitation ⚠️ Pragmatic Short-Term

**Approach:** Document the race condition as a known limitation for very fast typists. Recommend users use input-method-v2 if they hit this issue. Adjust marketing language accordingly.

**Implementation:**
- Add note to TROUBLESHOOTING.md
- Update README: "Fast typists on evdev may experience rare character duplication"
- Add FAQ entry
- Implement in documentation only (30 minutes)

**Pros:**
- ✅ Immediate (no code changes)
- ✅ Honest and transparent
- ✅ Users can self-serve (switch to input-method-v2)
- ✅ Doesn't block other features

**Cons:**
- ⚠️ Limits feature scope (evdev becomes "best-effort" not "guaranteed")
- ⚠️ Can't call evdev "production-ready"
- ⚠️ Affects power users (SRE scripts, rapid typing)
- ⚠️ Public record of known issue

**Recommendation:** Choose this if you want to ship quickly and accept the limitation.

### Option 3: Restrict to input-method-v2 Only 🔒 Most Conservative

**Approach:** Disable evdev expansion completely (keep capture, but disable matching). Force users to use input-method-v2 for expansions.

**Implementation:**
- Set expansion matching to no-op when evdev backend is active
- Users still get evdev capture, but must use input-method-v2 for output
- Requires config changes and backend coordination

**Pros:**
- ✅ Guarantees correctness (input-method-v2 has exclusive grab)
- ✅ No architecture changes needed
- ✅ Clear, simple message

**Cons:**
- ❌ Severely limits functionality (evdev expansion broken on many systems)
- ❌ Reduces user choice
- ❌ Contradicts current capability-based auto-selection
- ❌ Input-method-v2 has its own limitations (Escape key loss)

**Recommendation:** Choose this only if you want maximum safety over functionality.

## Recommendation Matrix

| Priority | Best Choice | Rationale |
|----------|-------------|-----------|
| **Correctness First** | Option 1 (Queuing) | Guarantees no data loss, even for edge cases |
| **Ship Quickly** | Option 2 (Documentation) | Honest, transparent, ships today |
| **Maximum Safety** | Option 3 (Restriction) | Most conservative, but limits features |

## Timeline Impact

- **Option 1 (Queuing):** 8-12 hours implementation + 2 hours testing = v1.2.1 (next sprint)
- **Option 2 (Documentation):** 30 minutes documentation + 1 hour testing = v1.2.0 (this week)
- **Option 3 (Restriction):** 3 hours implementation + 2 hours testing = v1.2.0 (this week)

## Next Steps

1. **Stakeholder Decision:** Choose Option 1, 2, or 3
2. **If Option 1:** Schedule 10-12 hours for implementation, testing, and verification
3. **If Option 2:** I can write documentation and test scenarios (30 min)
4. **If Option 3:** I can implement backend changes and tests (3-4 hours)
5. **After Decision:** Implement chosen approach and run full test suite
6. **Release:** Merge to main, tag v1.2.0 (or v1.2.1), update release notes

## Technical Details for Implementation (If Option 1 Chosen)

### Key-Event Queuing Architecture

```
Input Thread (evdev)              Matcher Thread
    |                                  |
    v                                  v
Raw keystroke ----[queue]----> Buffer K events
    |                              |
    |                              v
    |                          Process & decide
    |                              |
    |<---[signal ready]------------|
    |
    v
Forward to app (guaranteed after delete)
```

### Changes Required

1. **evdev backend (crates/backend-evdev/src/lib.rs):**
   - Add bounded event queue (Vec<KeyEvent> with max 32 events)
   - Add channel to signal matcher is ready
   - Wait for ready signal before forwarding to app

2. **daemon (crates/daemon/src/main.rs):**
   - Coordinate queuing with event loop
   - Send "ready" signal after expansion decision

3. **Tests:**
   - Add high-speed typing scenario test
   - Verify no duplication with back-to-back triggers
   - Benchmark latency impact

### Risk Assessment

- **Low risk:** Architecture change is localized to evdev backend
- **Mitigation:** Extensive testing with multiple typing speeds
- **Rollback:** Revert to current code if regressions found

---

## Questions for Stakeholder

1. Do you need correctness guarantee (Option 1), or is documented limitation acceptable (Option 2)?
2. Can you allocate 10-12 hours for implementation if Option 1 is chosen?
3. For Option 2: Is it acceptable to document this as a known limitation of evdev?
4. What's your timeline for v1.2.0 release?

**Decision Due:** [Your preferred date]  
**Contact:** stephan.loesevitz@gmail.com
