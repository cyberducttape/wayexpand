# P1 Architectural Improvements Roadmap (v1.3+)

> Historical design roadmap. Proposed APIs and status statements below describe
> the planning snapshot, not the current implementation. See
> [`BACKENDS.md`](BACKENDS.md), [`SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md), and
> [`SECURITY.md`](../SECURITY.md) for current status.

This document outlines P1 architectural debt that requires refactoring for full correctness and security.

## Current P1 Issues

### 1. Command Execution Policy Pre-Check (PARTIALLY FIXED v1.2)

**Status:** Intermediate fix in place; full solution deferred to v1.3

**Current Behavior:**
- Commands may execute before policy approval
- Pre-flight checks now prevent determinable violations
- Post-execution checks still needed for runtime constraints

**Example Scenario:**
```
User types trigger → engine.process() executes command → policy checked → expansion blocked
↑ Command has already run, side effects irreversible
```

**Intermediate Fix (v1.2):**
- `pre_flight_check()` catches disable_commands before processing
- Prevents some command execution before policy approval
- Logs violations in audit mode

**Proper Fix (v1.3+):**
1. Make command execution optional/deferred in engine
2. Return "pending" results without executing commands
3. Caller checks policy before executing deferred commands
4. Only commit expansion AFTER policy approval

**Implementation Steps:**
```rust
// Proposed v1.3+ architecture
pub struct PendingExpansionResult {
    pub trigger: String,
    pub matched_text: String,
    pub command: Option<CommandConfig>, // Not yet executed
    pub template: String,                 // Not yet rendered
}

// Engine returns pending results
pub fn process(&mut self, event: InputEvent) -> Vec<PendingExpansionResult>

// Caller decides execution:
for pending in results {
    if policy.allows(&pending) {
        pending.execute() // Now safe to execute
    }
}
```

---

### 2. Input-Method Key Press/Release Semantics (P1)

**Status:** Documented limitation; full fix requires state machine refactor

**Current Limitation:**
Only KEY_PRESS events are captured for unsupported keys. KEY_RELEASE events are not tracked, converting held keys into synthetic taps.

**Impact:**
- Held arrow navigation produces single tap (cursor moves once instead of continuously)
- Held Delete produces single deletion instead of repeated
- Key-repeat workflows broken
- Duration-sensitive applications affected

**Root Cause:**
```rust
// Current (line 452 of backend-input-method/src/lib.rs)
if key_state == wl_keyboard::KeyState::Pressed {  // ← Only PRESS
    state.pending_key_pass_through.push_back(...)
}
// RELEASE events ignored - not paired with PRESS
```

**Proper Fix (v1.3+):**
Implement press/release state machine with held-key tracking:

```rust
// Proposed state machine
struct HeldKey {
    keycode: u32,
    modifiers: Modifiers,
    press_time: Instant,
}

impl InputMethodSource {
    held_keys: HashMap<u32, HeldKey>,  // Track all held keys
    
    fn handle_press(&mut self, key: u32) {
        self.held_keys.insert(key, HeldKey { ... });
        // Forward to injector
    }
    
    fn handle_release(&mut self, key: u32) {
        if let Some(held) = self.held_keys.remove(&key) {
            // Forward RELEASE to injector
            // Properly end held-key sequence
        }
    }
    
    fn cleanup_on_disconnect(&mut self) {
        for key in self.held_keys.values() {
            // Force-release all held keys
        }
    }
}
```

**Implementation Steps:**
1. Track press/release pairs as state machine
2. Forward both PRESS and RELEASE to libei injector
3. Handle compositor repeat-info events
4. Force-release on disconnect/deactivate
5. Add tests for held-key sequences

**Verification:**
- Test held arrow navigation in multiple apps
- Test key-repeat with Delete key
- Test modifier+key held sequences
- Test cleanup on compositor loss

---

### 3. Release Script PKGBUILD Checksum Workflow (P1)

**Status:** Documented; workaround in place; proper fix deferred to v1.3

**Current Issue:**
Release checksums are computed AFTER tagging, making the tag stale:
```
Commit & tag → Push to GitHub → Compute checksum → Update PKGBUILD (stale tag)
```

**Proper Fix (v1.3+):**
Decouple version-only commit from checksum-bearing release:

```bash
# Step 1: Version-only commit and tag
git commit -m "release: version X.Y.Z"
git tag vX.Y.Z

# Step 2: Create source archive from tag
git archive --format tar.gz --prefix wayexpand-X.Y.Z vX.Y.Z -o wayexpand-X.Y.Z.tar.gz

# Step 3: Compute checksum
sha256sum wayexpand-X.Y.Z.tar.gz > checksum.txt

# Step 4: Update PKGBUILD with checksum
sed -i "s/sha256sums=.*/sha256sums=('$(cat checksum.txt | cut -d' ' -f1)')/" PKGBUILD

# Step 5: Create final release commit (includes checksum)
git commit --amend -m "release: version X.Y.Z (with checksum)"
# OR create separate commit
git commit -m "release: PKGBUILD checksums for X.Y.Z"

# Step 6: Push everything
git push origin main vX.Y.Z
```

**Implementation Steps:**
1. Modify `prepare-release.sh` to create source archive
2. Compute checksum before finalizing tag
3. Include checksum in release commit
4. Update GitHub release workflow to verify checksum

---

### 4. Pack Filtering Architecture (P1)

**Status:** Partially fixed (gates with safe_mode); full solution deferred to v1.3

**Current Behavior:**
Pack filtering happens inside `FleetConfig::apply_base_and_policy()`, which executes regardless of audit/safe_mode distinction.

**Intermediate Fix (v1.2):**
Gate filtering with `policy.safe_mode &&`:
- Audit mode: all packs retained, violations logged at runtime
- Safe mode: disallowed packs filtered

**Proper Fix (v1.3+):**
Move filtering to daemon level:

```rust
// Move from fleet.rs to daemon/main.rs
fn filter_packs_by_policy(config: &mut Config, policy: &OrganizationPolicy) {
    if !policy.safe_mode {
        return; // Audit mode: retain all for logging
    }
    if policy.allowed_packs.is_empty() {
        return; // No restriction
    }
    // Filter based on pack source tracking
}
```

**Benefits:**
- Cleaner separation of concerns
- Better audit trail visibility
- Fleet loader becomes pure configuration merge
- Daemon owns policy enforcement

---

## Implementation Priority for v1.3

1. **Command Execution Deferral** (highest priority - security)
   - Prevent side effects before policy check
   - Requires engine refactoring

2. **Input-Method Key State Machine** (high priority - correctness)
   - Fix held-key semantics
   - Required for production keyboard experience

3. **Release Checksum Workflow** (medium priority - process)
   - Eliminate stale tags
   - Improve release automation

4. **Pack Filtering Migration** (medium priority - architecture)
   - Move to daemon level
   - Cleaner separation

## Testing Strategy

Each P1 fix should include:
- Unit tests for new state machines
- Integration tests for policy enforcement
- End-to-end tests simulating real scenarios
- Matrix testing: policy mode × backend × use case

## Estimated Effort

- Command execution: 3-5 days (engine refactor)
- Key state machine: 2-3 days (state tracking + tests)
- Checksum workflow: 1 day (script changes)
- Pack filtering: 1 day (refactor)

**Total: 1-2 weeks for complete P1 resolution**

---

## See Also

- [SECURITY.md](../SECURITY.md) - Security model and guarantees
- [BACKENDS.md](BACKENDS.md) - Backend limitations and roadmap
- [PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md) - Full feature roadmap
