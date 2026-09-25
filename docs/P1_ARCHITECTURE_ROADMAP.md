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

**Status:** Implemented in the input-method source; compositor/client
certification remains outstanding.

Unsupported keys now use a lifecycle-aware state machine. Press and release
events are paired, repeat notifications do not synthesize extra taps while a
key is held, and held virtual keys are released during focus loss,
deactivation, transport failure, and source teardown.

The remaining risk is compositor-specific behavior: modifier identity,
repeat rates, shortcut timing, and reconnect behavior still require real
session certification.

**Implemented state machine:**

```rust
// Simplified shape of the implemented state machine
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

The implementation uses `StateData::virtual_held_keys` and queues both
`Pressed` and `Released` transitions for the libei injector. Contract tests
cover held arrows, held Delete/Backspace, repeat notifications, and cleanup.

**Verification:**
- Contract tests cover the lifecycle and cleanup invariants.
- Real compositor/client sessions are still required for release certification.

---

### 3. Release Script PKGBUILD Checksum Workflow (P1)

**Status:** Complete

`scripts/prepare-release.sh` now stages the release metadata, creates the
deterministic source archive from that staged tree, records its checksum in
`PKGBUILD`, commits the complete release state, and verifies that the final
commit reproduces the same archive before creating `vX.Y.Z`. The release
workflow independently verifies the source archive against the checksum in
`PKGBUILD`.

Packaging metadata is marked `export-ignore`, and Arch fetches the generated
release asset rather than a GitHub-generated archive. This avoids a checksum
cycle while keeping the release tag and published source archive reproducible.

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
