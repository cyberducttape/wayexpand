# Remaining P0 Blockers for v1.2 Release

This document lists the 6 critical issues that must be resolved before tagging v1.2. Each has implementation guidance and test cases.

## #15: Fix wayexpand doctor — Recognize evdev+libei as valid deployed state

**Problem:** 
- `wayexpand doctor` only reports capture ready when input-method-v2 probes successfully
- KDE Plasma requires `--source=evdev --backend=libei`, which doctor doesn't recognize as valid
- Users are told to wait for impossible condition (doctor reports "not ready" but evdev is actually working)

**Current behavior:**
```
$ wayexpand doctor
Capture readiness: NOT READY (evdev not recognized)
```

**Expected behavior:**
```
Input sources:
  input-method-v2      [unavailable]
  evdev                [ready]

Output backends:
  libei                [available; requires portal consent]
  wlroots vk           [unavailable]

Recommended deployment:
  evdev → libei (ready for use)
```

**Implementation:**
- File: `crates/daemon/src/diagnostics.rs` (or similar)
- Change: Evaluate complete source+backend combinations instead of just individual components
- Test: Run on KDE Plasma with evdev, verify it reports evdev+libei as deployable

**Definition of done:**
- doctor reports valid working combinations per compositor
- doctor no longer says "not ready" for evdev+libei on KDE
- CI validates doctor output on test systems

---

## #16: Mark GitHub /releases/latest as v1.1.2

**Problem:**
- GitHub's canonical `/releases/latest` still resolves to v0.2.1
- Users clicking "Latest Release" see ancient version
- Project appears unmaintained despite active development

**Current state:** v1.1.2 exists but isn't marked as latest

**Solution:** GitHub UI task (cannot automate)
1. Go to https://github.com/itchyitchy123/wayexpand/releases
2. Click "v1.1.2" release
3. Click "Edit"
4. Check "Set as the latest release"
5. Save

**Definition of done:**
- `/releases/latest` redirects to v1.1.2
- Public releases page shows v1.1.2 as latest

---

## #17: Fix diagnostics — Separate implementation status from environment status

**Problem:**
- Current diagnostics conflate "backend doesn't exist" with "permission denied"
- Example: uinput shows `RequiresPermission` even though it's not implemented
- Misleading users about what's actually available

**Current enum (incomplete):**
```rust
enum BackendState {
    Implemented,
    Available,
    Unavailable,
    NotImplemented,
    RequiresPermission,
}
```

**Needed design:**
```
Backend State (3 dimensions):
1. Implementation: [Implemented | NotImplemented]
2. Device/Protocol: [Present | Absent]
3. Permission: [Granted | RequiresPermission | Unavailable]
4. Connection: [Connected | Connecting | Unavailable]

Example output:
uinput
  Implementation: NotImplemented
  Device: Present
  Permission: N/A
  Status: Not available (implementation not complete)
```

**Files to change:**
- Diagnostics enum definition
- doctor output formatting
- JSON schema for doctor --json

**Test cases:**
- uinput (not implemented, device present)
- libei (implemented, requires portal)
- input-method-v2 (implemented, available on GNOME)
- evdev (implemented, available with permission)

**Definition of done:**
- Diagnostics output clearly distinguishes "not implemented" from "permission needed"
- No misleading "RequiresPermission" for unimplemented backends
- doctor --json has semantic separation

---

## #18: Fix COMPATIBILITY.md — Document actual diagnostic enum states

**Problem:**
- COMPATIBILITY.md documents non-existent enum values as "stable contract"
- Lists `Experimental` state that doesn't exist in code
- Omits actual states like `Available` and `NotImplemented`
- Creates false sense of API stability

**Current doc claims (wrong):**
```
Implemented
RequiresPermission
Unavailable
Experimental
```

**Actual enum (from code):**
```
Implemented
Available
Unavailable
NotImplemented
RequiresPermission
```

**Fix:**
1. Audit actual enum in `crates/daemon/src/diagnostics.rs`
2. Update COMPATIBILITY.md to document real states
3. Add contract tests that serialize/deserialize and verify docs match code
4. Mark section "Experimental" not "Stable" until fixed

**Definition of done:**
- COMPATIBILITY.md lists exactly the enum values in code
- Contract test ensures serialization matches docs
- No fictional states documented

---

## #19: Fix app-filter matching — Prefer app_id over window title

**Problem:**
- App filter matches window title even when app_id doesn't match
- Example: Konsole with title "Thunderbird troubleshooting" matches `app_filter = ["thunderbird"]`
- Unsafe — can expand sensitive snippets in wrong application

**Current logic:**
```
Does app_id contain filter string?
  YES → match
  NO → Does window title contain filter string?
       YES → match (WRONG!)
```

**Correct logic:**
```
Is app_id available?
  YES → Match ONLY if app_id contains filter string (exact path)
  NO  → Optionally fall back to title match (with warning)
```

**Files to change:**
- `crates/core/src/app_filter.rs` (matching logic)
- Docs about app-filter behavior

**Test cases:**
```
filter: ["thunderbird"]
app_id: "org.mozilla.Thunderbird" → MATCH ✓
app_id: "org.kde.konsole" with title "Thunderbird..." → NO MATCH ✓

filter: ["kde"]
app_id: "org.kde.dolphin" → MATCH ✓
app_id: "org.gnome.nautilus" with title "KDE docs" → NO MATCH ✓
```

**Definition of done:**
- App-id takes precedence over title
- Tests verify app mismatch doesn't match on title
- Behavior documented in app_filter section

---

## #20: Fix app-filter preview — Allow context selection for testing

**Problem:**
- App-filtered snippets can't be previewed in editor
- Engine fails closed (won't match) when active window unknown
- User sees "No expansion matched" and thinks snippet is broken
- But it actually works when triggered during real use

**Current behavior:**
```
User creates: app_filter = ["thunderbird"]
User clicks "Preview"
Engine: Can't determine active app → fails closed → "No match"
User: "This snippet doesn't work!" (wrong — it works when app is Thunderbird)
```

**Solution:** Add context selector to preview
```
Preview application:
  [ Ignore app restriction ▼ ]
  
Or:
  [ Thunderbird ▼ ]
  [ Konsole ]
  [ Firefox ]
  [ No active application ]
```

**Implementation:**
- GUI (egui): Add dropdown in preview panel
- CLI/TUI: Support `--preview-app=thunderbird` flag
- Pass simulated `WindowChanged` event to matcher with selected app_id

**Files to change:**
- `crates/gui/src/main.rs` (or UI module)
- `crates/ui/src/main.rs` (TUI)
- `crates/cli/src/main.rs` (CLI)

**Test cases:**
```
app_filter = ["thunderbird"]
Preview without context → "No match"
Preview with "thunderbird" selected → Match ✓
Preview with "konsole" selected → "No match"
```

**Definition of done:**
- App-filtered snippets can be previewed with app context
- User never sees false "no match" for valid app-filtered snippets
- Works in GUI, TUI, and CLI

---

## Implementation Priority

**Week 1 (P0 critical path):**
1. #15 - doctor fix (high impact, users currently blocked)
2. #16 - GitHub latest (quick win, high visibility)
3. #19 - app-filter matching (security issue)

**Week 2:**
4. #17-18 - diagnostics redesign (architectural, enables other work)
5. #21 - preview context (UX fix)

**Definition of "done" for v1.2:**
- All 6 P0 blockers resolved
- Tests added for each fix
- Documentation updated
- No regressions in CI

---

## Testing Checklist

After each fix, verify:
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] doctor output is accurate
- [ ] No regressions in other backends
- [ ] Documented in release notes
- [ ] Screenshots updated if UI changed
