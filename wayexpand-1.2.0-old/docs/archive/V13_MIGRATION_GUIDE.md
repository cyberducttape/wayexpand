# v1.3 Architecture Migration Guide

> Historical implementation plan. This checklist describes a proposed v1.3
> architecture, not the current code. Some APIs and flows have since changed;
> check `docs/BACKENDS.md`, `docs/SUPPORT_MATRIX.md`, and `SECURITY.md` for
> current behavior before using this as design guidance.

This archived guide preserves the proposed steps from the v1.2 planning period.
Its “current” and “target” architecture examples refer to that historical
snapshot, not the present implementation.

## 1. Command Execution Deferral (PRIORITY: CRITICAL)

### Overview
Move from immediate command execution to deferred execution with policy pre-approval.

### Current Architecture (v1.2)
```
engine.process(event) → ExpansionResult (commands already executed) → policy check → append to injection queue
↑ Side effects happened, can't be undone
```

### Target Architecture (v1.3)
```
engine.process(event) → PendingExpansionResult (commands NOT executed) → policy check → execute_with_policy() → injection queue
↑ Side effects deferred until policy approved
```

### Implementation Steps

#### Phase 1: Add Deferred Types (DONE in v1.2)
- ✅ `PendingExpansionResult` struct added to engine.rs
- ✅ `execute_with_policy()` method signature defined
- Remaining: Implement `run_command()` integration

#### Phase 2: Engine Refactoring (v1.3 TODO)
```rust
// OLD API (v1.2)
impl ExpansionEngine {
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        // Executes commands, returns final results
    }
}

// NEW API (v1.3)
impl ExpansionEngine {
    pub fn process_deferred(&mut self, event: InputEvent) -> Vec<PendingExpansionResult> {
        // Returns pending results without executing commands
    }
    
    // Keep old API for compatibility, but mark as deprecated
    #[deprecated(since = "1.3", note = "use process_deferred() instead")]
    pub fn process(&mut self, event: InputEvent) -> Vec<ExpansionResult> {
        // Internally uses process_deferred() + execute_with_policy()
    }
}
```

#### Phase 3: Caller Updates (v1.3 TODO)

**Daemon (crates/daemon/src/main.rs):**
```rust
// OLD
let results = engine.process(event);
for result in results {
    policy.check_and_log_violations(...);
    backend.inject(result)?;
}

// NEW
let pending = engine.process_deferred(event);
for pending in pending {
    policy.check_and_log_violations(...);
    if policy.safe_mode {
        continue; // Block execution if safe_mode violation
    }
    let result = pending.execute_with_policy()?;
    backend.inject(result)?;
}
```

**IBus Backend (crates/backend-ibus/src/lib.rs):**
```rust
// OLD
let results = self.engine.process(event);
for result in results {
    // Check policy post-execution
}

// NEW
let pending = self.engine.process_deferred(event);
for pending in pending {
    if let Some(violation) = self.policy.expansion_policy_violation(...) {
        if self.policy.safe_mode {
            continue; // Block BEFORE execution
        }
    }
    let result = pending.execute_with_policy()?;
    // Process result
}
```

#### Phase 4: Tests (v1.3 TODO)
```rust
#[test]
fn command_not_executed_before_policy_check() {
    // Verify command NOT executed during process_deferred()
    // Verify only execute_with_policy() executes the command
}

#[test]
fn safe_mode_blocks_command_execution() {
    // Verify command does NOT execute in safe_mode when policy forbids
}

#[test]
fn audit_mode_logs_but_executes() {
    // Verify audit_mode logs violation but still executes
}
```

### Historical Checklist

The unchecked list in the original plan is not a current work queue. A deferred
API exists, but command execution and policy behavior differ by caller; see the
current security documentation for its limits.

---

## 2. Input-Method Key Press/Release State Machine (PRIORITY: HIGH)

### Overview
Track and properly release held keys for proper keyboard semantics.

### Current Issues
- Only KEY_PRESS captured
- KEY_RELEASE not tracked
- Held keys become synthetic taps

### Target Architecture

```rust
// New in input-method backend
struct HeldKey {
    keycode: u32,
    modifiers: Modifiers,
    press_time: Instant,
}

pub struct InputMethodSource {
    // ... existing fields ...
    held_keys: HashMap<u32, HeldKey>,  // Track currently-held keys
}

impl InputMethodSource {
    fn on_key_press(&mut self, key: u32) {
        let held = HeldKey { keycode: key, ... };
        self.held_keys.insert(key, held);
        // Forward to injector
    }
    
    fn on_key_release(&mut self, key: u32) {
        if self.held_keys.remove(&key).is_some() {
            // Forward RELEASE to injector
            // Pair with PRESS for proper held-key sequence
        }
    }
    
    fn force_release_all(&mut self) {
        // Called on disconnect/deactivate
        for (_, held) in self.held_keys.drain() {
            // Force-release each held key
        }
    }
}
```

### Implementation Steps

#### Phase 1: Add State Tracking
1. Add `held_keys` field to `InputMethodSource`
2. Modify key event handler to process both PRESS and RELEASE
3. Forward RELEASE events to injector

#### Phase 2: Handle Repeats
1. Track repeat events from compositor
2. Forward repeats to injector
3. Ensure repeat semantics preserved

#### Phase 3: Cleanup
1. Add `force_release_all()` for disconnect handling
2. Call on deactivation/reconnection
3. Test with compositor loss scenarios

#### Phase 4: Tests
```rust
#[test]
fn held_arrow_key_produces_held_sequence() {
    // Press → multiple repeats → Release
    // Verify correct sequence sent to injector
}

#[test]
fn key_release_on_disconnect() {
    // Verify all held keys force-released on disconnect
}

#[test]
fn held_delete_produces_repeated_deletion() {
    // Verify repeated-delete semantic preserved
}
```

### Completion Checklist
- [ ] Add `held_keys` tracking structure
- [ ] Modify key event processing for RELEASE
- [ ] Forward RELEASE events to injector
- [ ] Handle compositor repeat-info
- [ ] Implement force-release cleanup
- [ ] Add state machine tests
- [ ] Test with real applications
- [ ] Update documentation

---

## 3. Release Script PKGBUILD Checksum Workflow (PRIORITY: MEDIUM)

### Overview
Compute and include PKGBUILD checksums before finalizing release tag.

### Current Issue
```
Commit → Tag → Push → Compute Checksum → Update PKGBUILD (stale tag)
```

### Target Process
```
Commit → Tag locally → Create archive → Compute checksum → Update PKGBUILD → Commit checksum → Push all
```

### Implementation

Modify `scripts/prepare-release.sh`:

```bash
# After version updates and before tagging
printf '%s\n' "Creating release archive and computing checksum..."

# Create source archive from HEAD
git archive --format tar.gz --prefix wayexpand-$new_version/ \
    --output wayexpand-$new_version.tar.gz HEAD

# Compute checksum
checksum=$(sha256sum wayexpand-$new_version.tar.gz | cut -d' ' -f1)

# Update PKGBUILD
sed -i.bak "s/sha256sums=.*/sha256sums=('$checksum')/" PKGBUILD
rm -f PKGBUILD.bak

# Verify checksum
test "$(sed -n 's/sha256sums=//p' PKGBUILD)" = "('$checksum')" || exit 1

# Stage checksum update
git add PKGBUILD

# Update version commit with checksum
git commit --amend -m "release: version $new_version (includes PKGBUILD checksum)"

# Tag should include checksum in release commit
git tag -d v$new_version 2>/dev/null || true
git tag -a "v$new_version" -m "Release v$new_version"

# Clean up archive
rm -f wayexpand-$new_version.tar.gz
```

### Verification
- [ ] Archive created successfully
- [ ] Checksum computed and inserted
- [ ] PKGBUILD validates with checksum
- [ ] Tag includes checksum in commit
- [ ] Test in clean repo

### Completion Checklist
- [ ] Modify prepare-release.sh
- [ ] Add archive creation step
- [ ] Compute and insert checksum
- [ ] Verify PKGBUILD integration
- [ ] Test release process end-to-end

---

## 4. Pack Filtering Migration to Daemon Level (PRIORITY: MEDIUM)

### Overview
Move pack filtering from fleet loader to daemon for cleaner architecture.

### Current State (v1.2)
- Fleet loader applies policy filtering
- Gated with `policy.safe_mode &&`
- Limited visibility in audit mode

### Target Architecture (v1.3)

#### Remove from fleet.rs
```rust
// DELETE this code from FleetConfig::apply_base_and_policy()
if !policy.allowed_packs.is_empty() && policy.safe_mode {
    fleet.config.expansion.retain(|expansion| { ... });
    fleet.config.hotkey.retain(|hotkey| { ... });
}
```

#### Add to daemon/main.rs
```rust
fn apply_pack_policy(config: &mut Config, policy: &OrganizationPolicy) {
    if !policy.is_active() || policy.allowed_packs.is_empty() {
        return;
    }
    
    // Track what would be filtered for audit logging
    let mut filtered_packs = Vec::new();
    
    config.expansion.retain(|expansion| {
        // Check if from disallowed pack
        // Log if audit mode
        // Return whether to keep
    });
    
    config.hotkey.retain(|hotkey| {
        // Same for hotkeys
    });
    
    if !policy.safe_mode {
        // In audit mode, log what was filtered
        for pack in filtered_packs {
            policy::log_violation(policy, &format!("pack '{}' is not in allowed list", pack));
        }
    }
}
```

#### Integration in daemon
```rust
let mut config = ReloadableConfig::load_with_policy(&path, policy.clone())?;

// Apply pack filtering at daemon level
apply_pack_policy(config.engine_mut().config_mut(), &policy);

// Now engine.process() has correct config
```

### Completion Checklist
- [ ] Remove filtering from fleet.rs
- [ ] Add `apply_pack_policy()` to daemon
- [ ] Call after config load
- [ ] Verify pack provenance tracking works
- [ ] Test audit mode visibility
- [ ] Test safe mode filtering
- [ ] Update tests

---

## Migration Testing Strategy

### Unit Tests
```rust
// Command deferral
test_command_deferred_not_executed()
test_safe_mode_blocks_execution()
test_audit_mode_logs_and_executes()

// Key state machine  
test_held_key_press_release_pair()
test_key_repeat_preserved()
test_force_release_on_disconnect()

// Pack filtering
test_pack_filtering_at_daemon_level()
test_audit_mode_visibility()
test_safe_mode_enforcement()
```

### Integration Tests
```rust
// Full flow
test_command_execution_with_policy_approval()
test_held_key_navigation_works()
test_pack_filtering_prevents_disallowed_expansions()
```

### End-to-End Tests
```bash
# Test actual user scenarios
test_held_arrow_navigation.sh
test_command_execution_blocked_in_safe_mode.sh
test_pack_visibility_in_audit_mode.sh
```

---

## Implementation Order

1. **Phase 1 (Week 1):** Command execution deferral + tests
2. **Phase 2 (Week 2):** Key state machine + tests
3. **Phase 3 (Week 3):** Pack filtering migration + tests
4. **Phase 4 (Week 3):** Release script updates + end-to-end testing

**Total Estimated Effort:** 2-3 weeks
**Blockers:** None (all work is parallel after Phase 1)

---

## Verification Criteria

- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] Held key navigation works in multiple apps
- [ ] Command execution respects safe_mode
- [ ] Pack filtering has audit visibility
- [ ] Release process produces correct checksums
- [ ] No regressions in existing functionality
- [ ] Documentation updated

---

## Rollback Plan

If migration encounters unforeseen issues:
1. Keep old APIs as deprecated (don't remove)
2. Implement new APIs in parallel
3. Gradually migrate callers
4. Full removal of old APIs only after v1.3.1 stable
