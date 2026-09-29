# Code Organization Refactoring Plan

## Overview
Address the large monolithic source files that obscure integration boundaries and make bugs harder to catch.

## Target Files (by priority)

### 1. `crates/daemon/src/main.rs` (2,318 lines) - HIGHEST PRIORITY
Split into:
- **input_loop.rs** (400 lines) - Input handling and reconnection logic
- **output_loop.rs** (350 lines) - Output injection and backend lifecycle
- **backend_lifecycle.rs** (300 lines) - Backend initialization and reconnection
- **main.rs** (600 lines) - Orchestration and startup

#### input_loop.rs Functions
- `connect_input_method_session()` - Input-method setup
- `connect_input_method_with_retry()` - Input-method reconnection with backoff
- `connect_evdev_with_retry()` - Evdev setup
- `wait_for_retry()` - Backoff logic
- `input_poll_interval()` - Adaptive polling

#### output_loop.rs Functions
- `connect_output_backend()` - Output selection
- `connect_output_with_retry()` - Output reconnection
- `OutputConnectError` struct

#### backend_lifecycle.rs Functions
- `spawn_window_tracker()` - Window tracking startup
- `drain_pending_window_events()` - Window event processing
- Window tracking state management

#### main.rs (Orchestration)
- `main()` function (heavily refactored for clarity)
- Policy loading
- Configuration management
- Signal handling
- Status publishing
- The event loop orchestration

### 2. `crates/core/src/engine/mod.rs` (2,268 lines) - MEDIUM PRIORITY
Split into:
- **engine/mod.rs** (target ~300 lines) - Public API, core types
- **engine/matching.rs** (400 lines) - Trigger matching logic
- **engine/command_runtime.rs** (350 lines) - Async command execution
- **engine/transaction.rs** (300 lines) - Undo/expansion state
- **engine/expansion.rs** (400 lines) - Expansion computation

### 3. Other Large Files (LOW PRIORITY - can be addressed later)
- `crates/cli/src/main.rs` (2,681 lines) - CLI logic can move to separate modules
- `crates/gui/src/main.rs` (2,150 lines) - GUI initialization and main loop
- `crates/backend-input-method/src/lib.rs` (2,418 lines)
- `crates/backend-libei/src/lib.rs` (1,649 lines)
- `crates/core/src/config.rs` (1,603 lines)

## Benefits

1. **Explicit Integration Boundaries**: The P0 async invalidation bug and undo corruption bug were both at daemon/engine boundaries. Smaller files make these boundaries visible.

2. **Easier Code Review**: Smaller files = easier to understand + harder to hide bugs

3. **Better Testing**: Clear module boundaries enable focused unit testing at integration points

4. **Reduced Cognitive Load**: 600-line files are easier to understand than 2,600-line files

## Current implementation status

The daemon lifecycle work is partially complete: input polling, output
connection, and window/backend lifecycle helpers live in their dedicated
modules. Event dispatch remains intentionally in `main.rs` because it owns the
evdev release/quiet-period gate and the orchestration of completed async
commands. An earlier unintegrated copy of that logic was removed; keeping two
dispatch implementations made race fixes easy to miss and created a false
refactoring target.

The core engine now also has dedicated `matching`, `command_runtime`,
`transaction`, and `expansion` modules, while `engine/mod.rs` remains the
coordinator and public API surface.

## Implementation Strategy

### Phase 1: Daemon Refactoring (Next Sprint)
1. Keep event processing in `main.rs` until its evdev gating and completion
   orchestration can move together without duplicating state transitions.
2. Create `input_loop.rs` ✅
3. Create `output_loop.rs` ✅
4. Create `backend_lifecycle.rs` ✅
5. Update main.rs to use new modules ✅ (window-event draining now has one path)
6. Run full test suite
7. Verify no behavioral changes

### Phase 2: Engine Refactoring (Following Sprint)
1. Create `engine/` directory
2. Extract modules one at a time
3. Update public API in engine/mod.rs
4. Run all engine tests after each module extraction

### Phase 3: Other Files (As Needed)
Address remaining large files based on priority and maintenance burden

## Testing Strategy

1. **Run full test suite after each module extraction**: `cargo test --locked --workspace`
2. **Verify no behavioral changes**: Existing tests should pass unchanged
3. **Add integration tests** for extracted modules as needed
4. **Code review**: Each extraction should be reviewed for clarity

## Expected Outcomes

✅ Improved code clarity and maintainability  
✅ Earlier detection of integration boundary bugs  
✅ Better onboarding for new contributors  
✅ Foundation for future enhancements  

## Notes

- All functions remain public during extraction (visibility can be refined later)
- Each extraction is a separate commit for easy review/revert
- No new features during refactoring - only reorganization
- All existing tests must pass without modification

## Estimated Timeline

- **Daemon refactoring**: 2-3 hours (split across 2-3 commits)
- **Engine refactoring**: 2-3 hours (split across 4-5 commits)
- **Other files**: 2-3 hours each as needed
- **Total**: 6-10 hours for complete refactoring

---

## Next Steps

1. If event dispatch is extracted, move the evdev gate, follow-up replay, and
   completed-command drain in the same change.
2. Split only when the module boundary preserves the single event-ordering
   point and its regression tests.
3. Reduce `main.rs` orchestration only after the behavior is covered by daemon
   integration tests.
4. Run tests and verify no regressions after each extraction.
