# v1.3 Critical Regressions - FIXED

> Historical regression analysis. The initial assessment below records bugs
> before fixes landed; its reproduction snippets and “currently passes” claims
> are not descriptions of current behavior. Use the repository's current tests
> and `SECURITY.md` for the supported contract.

## Executive Summary

The v1.3 deferred execution implementation introduced three critical architectural regressions in command execution policy enforcement.

**Status:** ✅ ALL FIXED (2026-09-24) through unified command execution architecture refactor (5 phases)

### How They Were Fixed

**Bug #2 (Async Worker Fallback Policy Bypass):** Fixed by enforcing `disable_commands` policy in sync fallback path + setting `command_backed` correctly

**Bug #3 (command_backed Flag Accuracy):** Fixed by setting `command_backed = expansion.command.is_some()` instead of hardcoding to false

**Bug #1 (Output Size Policy):** Fixed by introducing postflight policy that validates output size after command execution

**See:** [[P2_unified_command_execution_completion.md]](../../../memory/P2_unified_command_execution_completion.md) for implementation details

---

## Bug #1: Output Size Policy Check Happens Pre-Execution

### Problem

Policy checks `template_text.len()` (which is 0 if template is empty) but command output can exceed `max_replacement_size`.

```
Configuration:
  replacement = ""
  command = { program = "big-output" }
  max_replacement_size = 1000

Execution flow:
  Policy checks: replacement_size = 0 bytes ✓ PASS
        ↓
  run_command() executes
        ↓
  stdout = 500 KB
        ↓
  Inject 500 KB ✗ VIOLATES max_replacement_size
```

### Root Cause

`PendingExpansionResult::execute_with_policy()` calls `run_command()` directly without post-execution size validation.

### Design Problem

The documentation claim "commands NEVER execute before policy approval" (V13_COMPLETION_STATUS.md:22-23) is **false**. Some policy constraints are:
- **Knowable pre-execution:** `disable_commands`, backend restrictions, `require_absolute_commands`
- **Unknowable pre-execution:** actual output size, output validity (UTF-8)

### Correct Architecture

```
PRE-EXECUTION checks (in policy):
  ✓ disable_commands
  ✓ backend_allowed
  ✓ require_absolute_commands
  
POST-EXECUTION checks (after run_command()):
  ✓ result.insert.len() <= max_replacement_size
  ✓ output is valid UTF-8
```

### Test Case Needed

```rust
#[test]
fn output_size_policy_must_be_checked_post_execution() {
    // Template is empty (0 bytes), command outputs 100KB
    // max_replacement_size = 1KB
    // Historical expected behavior: blocked after command output is checked
}
```

---

## Bug #2: Async Worker Fallback Silently Bypasses `disable_commands`

### Problem

When async workers fail to initialize, the daemon silently falls back to synchronous command execution **with policy enforcement disabled**.

```
Daemon startup:
  if !config.engine.enable_async_commands() {
      warn!("... using sync command fallback");
  }
  
take_match() execution path:
  if self.config.organization.disable_commands {  // ← Check is here
      return None;
  }
  
But:
  } else if let (Some(runtime), Some(command)) = (...) {
  // This branch requires runtime.is_some()
  
  } else {
      self.render_expansion(config_index).ok()?
      // ↓ Falls through to SYNC execution
      // ↓ disable_commands check SKIPPED
      // ↓ run_command() executes synchronously in event path
```

### Why Test Didn't Catch It

```rust
#[test]
fn commands_execute_when_not_disabled() {
    let mut engine = ExpansionEngine::new(config)?;
    engine.enable_async_commands();  // ← MASKS THE BUG
    // Test passes because runtime exists
}
```

The test explicitly enables async workers, so it never exercises the fallback path.

### Worse: Result Metadata is Wrong

When sync fallback executes, the result is constructed with:
```rust
command_backed: false
```

So `apply_results()` thinks the data came from a static expansion, not a command.

### Design Problem

Worker startup failure is **not theoretical**:
- Task limit exhaustion (EAGAIN)
- System resource constraints
- Thread creation failures

Current behavior: silently reduce security guarantees.

**Correct behavior:** fail closed.

```
If async workers unavailable:
  ✓ Static expansions: continue working
  ✓ Command expansions: become unavailable
  ✓ User sees clear error (not silent fallback)
  ✓ Security semantics NEVER change
```

### Test Case Needed

```rust
#[test]
fn async_worker_failure_must_block_commands_not_fallback_to_sync() {
    let config = ...; 
    config.organization.disable_commands = true;
    
    let mut engine = ExpansionEngine::new(config)?;
    // DON'T call enable_async_commands() - simulate worker failure
    
    let results = engine.process(InputEvent::Text(":cmd".into()));
    
    // Should be empty: commands blocked by policy
    // Currently: may have result (sync fallback executed)
    assert!(results.is_empty(), "KNOWN BUG: disable_commands skipped in fallback");
}
```

---

## Bug #3: `command_backed` Flag Accuracy in Fallback Path

### Problem

When sync fallback executes a command, the result has:
```rust
command_backed: false  // ← LIE
```

This misleads `apply_results()` into thinking the data is safe (static expansion).

### Impact

Policy checks downstream use `command_backed` to determine if output needs size validation:
```rust
// In apply_results()
policy::check_and_log_expansion_violations(
    policy,
    result.insert.len(),
    result.command_backed,  // ← Used to gate output-size checks
    backend,
);
```

With `command_backed = false`, size checks may not happen.

### Test Case Needed

```rust
#[test]
fn command_backed_flag_must_reflect_actual_execution() {
    // When sync fallback executes command:
    // Expected: command_backed = true
    // Actual: command_backed = false
}
```

---

## Fix Strategy

### Phase 1: Stop the Bleeding (IMMEDIATE)

1. **Revert false documentation claims:**
   - V13_COMPLETION_STATUS.md:22-23 - "NEVER execute"
   - V13_COMPLETION_STATUS.md:50 - same claim
   - V13_COMPLETION_STATUS.md:201 - same claim

2. **Write regression tests** that document all three bugs

3. **Tag this branch as "known regressions"** - do not merge to main yet

### Phase 2: Architectural Fix (THIS WEEK)

#### Fix #1: Add Post-Execution Policy Checks

```rust
pub fn execute_with_policy(self) -> Result<ExpansionResult, CommandError> {
    let output = run_command(&self.command)?;
    
    // ✓ NEW: Post-execution validation
    if output.len() > MAX_SIZE {
        return Err(CommandError::OutputTooLarge);
    }
    if !is_valid_utf8(&output) {
        return Err(CommandError::InvalidUtf8);
    }
    
    Ok(ExpansionResult {
        insert: output,
        command_backed: true,
        ...
    })
}
```

#### Fix #2: Fail Closed on Worker Initialization Failure

```rust
// In daemon startup
if !config.engine.enable_async_commands() {
    // Don't silently continue
    error!("async command workers could not initialize; command-backed expansions are unavailable");
    
    // Option A: disable all commands (fail closed)
    config.organization.disable_commands = true;
    
    // Option B: reject startup entirely
    return Err("worker initialization failed");
}
```

#### Fix #3: Fix `command_backed` Flag in Sync Fallback

If fallback must exist (not recommended):
```rust
command_backed: true,  // ← Fix: was false
```

### Phase 3: Verify (NEXT REVIEW)

- [ ] All three regression tests pass
- [ ] `--locked` builds pass
- [ ] `cargo fmt` passes
- [ ] `cargo clippy` passes
- [ ] Full test suite passes
- [ ] No new security warnings

---

## Impact Assessment

### Severity: CRITICAL

- **Confidentiality:** N/A (expansion text is user data)
- **Integrity:** HIGH - Commands can produce unchecked output
- **Availability:** MEDIUM - Silent sync execution masks worker failure

### Affected Code Paths

1. **Daemon:** `apply_pending_results()` + `take_match()`
2. **IBus Backend:** pending result handler + `take_match()`
3. **Core Engine:** `PendingExpansionResult::execute_with_policy()`

### Why This Matters

Users or admins setting `max_replacement_size = 1000` expect 1KB limit to prevent DoS. An unchecked 500KB output violates that contract, with security implications for:
- Terminal/TUI systems with bounded buffers
- Mobile/resource-constrained environments
- Fleet/managed deployments with strict output policies

---

## References

- V13_COMPLETION_STATUS.md (contains false claims to revert)
- crates/core/src/engine.rs (PendingExpansionResult, take_match)
- crates/daemon/src/main.rs (apply_pending_results)
- crates/backend-ibus/src/lib.rs (pending result handler)

---

## Lesson Learned

Architectural refactoring that moves logic out of tested paths can silently break guarantees. The async-worker infrastructure existed and worked correctly. Moving command execution into the caller context introduced:
1. Policy timing mismatches (pre vs. post)
2. Fallback path that bypasses checks
3. Metadata accuracy loss

**Next time:** Keep command execution in the engine's existing async-worker path. The policy gate can still happen pre-execution, but command execution remains within the bounded, tested queue.
