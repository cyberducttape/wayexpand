# P2 Improvements Roadmap (v1.3+)

This document outlines architectural improvements identified during the comprehensive security audit but deferred due to complexity and release constraints.

## Priority: HIGH

### 1. Command Queue Observability — RESOLVED

**Problem:** Silent command failures when job queue saturates (16 capacity).

**Resolution:** Queue rejection, timeout, and command failure counters are
exposed through `wayexpand status`, and the daemon logs queue-rejection
warnings. Queue failures also restore the pending match instead of silently
losing it.

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

### 3. Config Reload Change Detection — RESOLVED

**Resolution:** Reload fingerprints use deterministic FNV-1a over the complete
file contents, alongside metadata checks. This avoids the old commutative
additive checksum and detects same-size/content changes reliably without
adding a cryptographic dependency.

**Impact:** Negligible in practice (user would have to intentionally craft collisions), but better for high-assurance scenarios.

---

### 4. Evdev Device Discovery Efficiency — RESOLVED

**Resolution:** Input polling is independent from device discovery. Known
devices are refreshed on a 30-second fallback interval, rather than on every
latency-sensitive poll. A udev monitor remains a possible future enhancement.

**Impact:** Reduces syscall volume dramatically during idle periods.

---

## Lower Priority

### 5. Config Reload Documentation/Behavior Mismatch — RESOLVED

The evdev discovery documentation now describes periodic refresh behavior.

---

### 6. Disable Title Matching Semantics — RESOLVED

`disable_title_matching=true` disables title fallback while retaining app-id
matching and failing closed when no app-id is available. Regression tests cover
both paths.

---

### 7. Pack Implementation Structure

**Problem:** Documentation structure doesn't match code behavior.

- Docs say: Create `~/.local/share/wayexpand/packs/my-pack/` with TOML inside
- Code does: Scan `packs/` for `.toml` files directly + subdirectories
- Provenance mismatch: Says "pack" but filter expects "pack:<name>"

**Solution:** Clarify intended structure and make documentation precise.

---

## Current follow-up candidates

- Add udev-backed evdev hot-plug notifications.
- Clarify pack layout and provenance documentation.
- Consider reducing the remaining synchronous/deferred matcher duplication.
