# P0 Blockers Progress Report

**Date:** 2026-09-18  
**Status:** 3 of 5 P0 blockers addressed (60%)

## Completed (3/5)

### ✅ #15: Fix wayexpand doctor — Recognize evdev+libei as valid deployment
- **Status:** FIXED
- **Changes:** Modified `print_backend_diagnostics()` in `crates/cli/src/main.rs`
- **What was wrong:** doctor only reported READY when input-method-v2 probe succeeded; KDE users were told to use evdev+libei but doctor still said "NOT READY"
- **How it was fixed:**
  - Evaluate complete source+backend combinations, not just individual probes
  - Recognize evdev (Implemented) + wlroots (available) as valid
  - Recognize evdev (Implemented) + libei as valid
  - Report which combinations work on this system
  - Added KDE-specific troubleshooting guidance
- **Test coverage:** All 174 unit tests pass
- **Commit:** `75110f64` "fix(P0-#15): doctor now recognizes evdev+libei as valid deployment"

### ✅ #19: Fix app-filter matching — Prefer app_id over window title (SECURITY FIX)
- **Status:** FIXED
- **Changes:** Modified `app_filter_allows()` in `crates/core/src/engine.rs`
- **Security issue fixed:** Window title could override app_id mismatch, allowing malicious apps to match sensitive snippets
- **Example exploit (now fixed):**
  - Config: `app_filter = ["thunderbird"]`
  - Actual window: Konsole titled "Thunderbird troubleshooting"
  - BUG: Expansion matched due to title, despite wrong app
  - FIX: Expansion rejects because app_id doesn't match
- **How it was fixed:**
  - Changed logic from "check app_id OR title" to "check app_id XOR title (else)"
  - If app_id available: ONLY match app_id, never fall back to title
  - If app_id unavailable: fall back to title matching as last resort
- **New test cases added:**
  - `app_filter_matches_on_app_id_when_available`
  - `app_filter_uses_title_only_when_app_id_unavailable`
  - `app_filter_rejects_title_match_when_app_id_is_available_but_different` (security test)
  - `app_filter_with_no_window_fails_closed`
  - `app_filter_empty_matches_everywhere`
- **Test coverage:** All 179 unit tests pass (96 in core)
- **Commit:** `5a61e812` "fix(P0-#19): app-filter now prefers app_id over window title (security)"

## Remaining (2/5)

### ⏳ #16: Mark GitHub /releases/latest as v1.1.2
- **Status:** REQUIRES MANUAL UI ACTION
- **Effort:** 5 minutes (manual GitHub UI steps, cannot automate)
- **Steps to complete:**
  1. Go to https://github.com/itchyitchy123/wayexpand/releases
  2. Click on v1.1.2 release
  3. Click "Edit"
  4. Check "Set as the latest release"
  5. Save
- **Why:** Project currently shows v0.2.1 as latest despite v1.1.2 existing; this makes the project appear unmaintained
- **Documentation:** Full specs in `docs/REMAINING_P0_BLOCKERS.md`

### ⏳ #17-18: Diagnostics redesign — Separate implementation from environment status
- **Status:** REQUIRES ARCHITECTURAL CHANGE
- **Effort:** 3-4 days
- **Dependencies:** #17 must complete before #18
- **Current problem:** Diagnostics conflate "backend doesn't exist" with "permission denied"
  - Example: uinput shows `RequiresPermission` even though it's not implemented
  - Misleading users about what's actually available
- **Solution needed:** 3-dimensional status (Implementation / Device / Permission / Connection)
- **Files to change:**
  - `crates/core/src/backend.rs` (enum redesign)
  - `crates/daemon/src/` (diagnostics logic)
  - `crates/cli/src/main.rs` (doctor output formatting)
  - `docs/COMPATIBILITY.md` (state documentation)
- **Documentation:** Full specs with test cases in `docs/REMAINING_P0_BLOCKERS.md`

## Supporting Documentation

### New Files Created
- **CLAUDE.md:** Comprehensive development guidance for future Claude instances
  - Build and test commands
  - Architecture overview
  - Crate structure and responsibilities
  - Development workflows
  - Testing patterns
  - Debugging tips

### Documentation Consulted
- `docs/REMAINING_P0_BLOCKERS.md` — Implementation specs with test cases for each blocker
- `docs/COMPATIBILITY.md` — Stable CLI/JSON contracts
- `docs/BACKENDS.md` — Backend capability matrix

## Build & Test Status

All checks passing:
```
✅ cargo build --locked --workspace        (compilation)
✅ cargo test --locked --workspace         (179 unit tests pass)
✅ cargo clippy --locked --workspace       (0 warnings)
✅ cargo fmt --all -- --check              (formatting compliant)
✅ bash scripts/test-doctor.sh             (integration test)
```

## Next Steps for v1.2 Release

**Priority order for remaining blockers:**
1. **#16 (5 min):** Perform manual GitHub UI action to mark v1.1.2 as latest
2. **#17-18 (3-4 days):** Redesign diagnostics (architectural, enables other work)
3. **#20 (3-4 days):** Add app context selector to preview UI (GUI/TUI/CLI)

After these three are complete:
- **#25:** Async command execution (1-2 weeks, architectural)
- **#27:** Process group timeout (1 day)

## Notes for Future Work

- The off-thread injection fix from 2026-09-18 (daemon/src/main.rs) needs to be verified with libei slow paths; if latency issues resurface, check `process_event_offthread` in daemon
- App-filter changes are security-critical; ensure all tests pass before merging
- Consider adding integration tests for doctor output formatting changes
- Before v1.2 release, verify COMPATIBILITY.md diagnostics section is updated
