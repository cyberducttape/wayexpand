# P2 Improvements Roadmap (v1.3+)

This document outlines architectural improvements identified during the comprehensive security audit but deferred due to complexity and release constraints.

## Priority: HIGH

### 1. Command Queue Observability

**Problem:** Silent command failures when job queue saturates (16 capacity).

**Symptoms:**
- User types trigger, nothing happens
- No error signal
- Text state diverges from what app contains
- No way to diagnose via logs/metrics

**Root Cause:**
```rust
runtime.sender.try_send(job).ok()?;  // Line 690 in engine.rs
```
Silently swallows `SendError` when queue full.

**Solution:**
- Add tracing::warn! when try_send fails
- Emit metrics: `command_queue_full_total`, `command_timeout_total`, `command_execution_failed_total`
- Document in `wayexpand status` command
- Consider multi-worker pool instead of single command worker (eliminates head-of-line blocking)

**Impact:** Production debugging becomes possible; single 5-second command no longer blocks all other command-backed expansions.

---

### 2. Process Cleanup on Successful Exit — RESOLVED

**Problem:** Process descendants survived when the direct child exited with
status 0.

**Resolution:** `run_command()` and `execute_hotkey()` now terminate the
process group after every ordinary child exit, including successful and
non-zero exits. The regression test
`process_descendants_cleaned_up_on_successful_exit` is enabled and verifies
that a descendant cannot continue running after successful completion.

**Impact:** Prevents resource leaks, especially in unsandboxed CLI/GUI preview
execution paths.

---

### 3. Config Reload Change Detection

**Status:** PARTIALLY FIXED in commit 28f247b

**Remaining Issue:** Fingerprint uses simple additive checksum, not cryptographic hash.

**Current:** `wrapping_add(u64)` summing 8-byte chunks
**Proposed:** BLAKE3 for strong change fingerprint

**Why:** On coarse-timestamp filesystems with malicious edits, simple checksum could theoretically have collisions. BLAKE3 is fast and cryptographically strong.

**Impact:** Negligible in practice (user would have to intentionally craft collisions), but better for high-assurance scenarios.

---

### 4. Evdev Device Discovery Efficiency

**Problem:** Re-enumerates `/dev/input/*` every 500ms polling cycle while idle.

**Root Cause:**
```rust
// In evdev polling loop
loop {
    // Re-discovers devices every iteration
    for entry in fs::read_dir("/dev/input/")? {
        // Open, probe, add to device list
    }
}
```

**Solution:**
- **Primary:** Use `udev` monitor for add/remove events
- **Fallback:** Slow polling (minutes, not 500ms) with device path caching
- **Skip:** Don't re-open already-known devices

**Impact:** Reduces syscall volume dramatically during idle periods.

---

## Lower Priority

### 5. Config Reload Documentation/Behavior Mismatch

Some docs claim devices are discovered at startup and retained, but polling loop rescans.

**Solution:** Either update docs OR refactor to match promised behavior (likely the former).

---

### 6. Disable Title Matching Semantics

**Problem:** `disable_title_matching=true` doesn't just "disable titles" - it disables entire app_filter.

**Current Behavior:** Matches ALL apps when this flag is set
**Intuitive Behavior:** Match only app_id, fail-closed if unavailable

**Solution (Breaking Change for v1.3+):**
Replace single boolean with explicit modes:
```toml
[organization]
app_filter_mode = "app_id_only"      # Fail-closed without app_id
app_filter_mode = "app_id_or_title"  # Current default
app_filter_mode = "disabled"         # Disable all app filtering
```

Requires migration docs for existing `disable_title_matching` users.

---

### 7. Pack Implementation Structure

**Problem:** Documentation structure doesn't match code behavior.

- Docs say: Create `~/.local/share/wayexpand/packs/my-pack/` with TOML inside
- Code does: Scan `packs/` for `.toml` files directly + subdirectories
- Provenance mismatch: Says "pack" but filter expects "pack:<name>"

**Solution:** Clarify intended structure and make documentation precise.

---

## Implementation Priority for v1.3

1. **Command queue observability** (HIGH) - unblocks production debugging
2. **Process cleanup** (HIGH) - fixes regression test
3. **Evdev efficiency** (MEDIUM) - improves idle behavior
4. **App filter semantics** (MEDIUM) - breaking change, needs migration path
5. **Config reload hash** (LOW) - theoretical improvement
6. **Pack structure** (LOW) - documentation only

---

## Metrics to Track (v1.3+ instrumentation)

```
command_queue_full_total: Counter of failed job submissions
command_timeout_total: Counter of commands exceeding timeout
command_execution_failed_total: Counter of subprocess failures
command_execution_slow_total: Counter of commands > 1 second
expansion_dropped_total: Counter of expansions lost to queue saturation
```

Dashboard: `wayexpand status --metrics-json` (future enhancement)
