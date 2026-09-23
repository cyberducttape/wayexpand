# Audit Findings: Code Quality, Risks, and Path to Legendary Status

Date: 2026-09-22

## Executive Summary

WayExpand's engineering foundation is strong: modular architecture, security-conscious design, comprehensive error handling, and honest documentation of Wayland limitations. The remaining gaps are not architectural flaws but execution challenges: real-world compositor certification, the unresolved secure-vs-fidelity capture trade-off, distribution/packaging reach, and IME support.

---

## Known Limitations (Documented, Not Bugs)

### 1. No Compositor Certified by Automated End-to-End Tests

**Impact:** Every new user must run `doctor` and hope; deployment on unfamiliar compositors is a gamble.

**Current Status:**
- Window tracking: solid on KDE via KWin scripting; wlroots path (Sway, Hyprland, river) is implemented but needs real-world certification; GNOME has no usable protocol path
- Capture/inject: tested at unit level; system-level behavior varies by compositor, kernel version, and display server implementation

**What's needed:**
- Automated end-to-end tests for Sway, Hyprland, KDE Plasma, GNOME covering:
  - Normal typing and Unicode (including emoji, RTL)
  - Password field suppression (where applicable)
  - Focus changes mid-expansion
  - Rapid-fire triggers and undo behavior
  - Application shortcut conflicts
  - Accessible matrix: documented compositor versions + test date + pass/fail status
- Published "certified" badge on supported combinations
- First-run experience that detects compositor and recommends optimal backend with explicit trade-offs

### 2. Fundamental Capture Trade-Off: Security vs. Fidelity

**The Problem:**
- **input-method-v2:** Respects sensitive field signals and password protection, but exclusive keyboard grab cannot safely pass through Escape, arrow keys, and function keys (users on GNOME report lost navigation)
- **evdev:** Preserves all keyboard events including Escape/arrows/F-keys, but reads all global keystrokes (requires `input` group), ignores password field signals, and needs careful timing/reinsert logic

**Current Mitigation:** Explicit user opt-in with clear warnings on mode selection; behavior is honest in docs but creates tension between power users and security-conscious users

**Research Directions:**
- Compositor-provided focused-window input filtering (systemd secure input, input ACLs)
- Improved input-method protocol support in compositors (GNOME, wlroots)
- Better portal/ACL integration for libei (portal token persistence is partially open; see PROFESSIONAL_ROADMAP.md #28)
- Constrained privileged helper (extreme complexity; not recommended without research first)

### 3. IME / Preedit / Composition Unsupported

**Impact:** CJK (Chinese, Japanese, Korean), many European composition workflows (ä, é, ç via dead keys or Compose), and IME-dependent input methods are second-class experiences or non-functional.

**Current Status:** Explicitly documented as unsupported; no platform-specific integration exists

**What's Needed:**
- Research toolkit integration (e.g., IBus, Fcitx on Linux; Wayland preedit if it materializes)
- Interim: clear documentation + "finish composition, then expand" workflow with optional hotkey
- Accessibility audit for RTL input

### 4. Window Tracking Gaps

**Status:**
- KDE: ✅ Solid (KWin D-Bus scripting)
- Sway/Hyprland/river: ✅ Implemented (wlroots `wlr-foreign-toplevel-management-v1`), needs real-world certification
- GNOME/Mutter: ❌ No usable protocol path exists (see `docs/GNOME_WINDOW_TRACKING.md`)

### 5. Portal Token Persistence (libei)

**Status:** Scaffolded; explicit consent dialog appears on each connection. User-initiated restore token persistence is the v1.2.1 roadmap item; see PROFESSIONAL_ROADMAP.md #28.

### 6. ARM64 Packaging

**Current Status:** Release workflow had to revert matrix work. Native aarch64 binaries can be built locally; official release coverage and package-manager distribution are incomplete.

---

## Code-Level Observations: Performance and Complexity

### 1. Matcher Node Performance

**Current Implementation:** `HashMap<char, Node>` for trie structure

**Assessment:**
- **Fine for typical use:** Most snippet libraries (100-1000 entries) are unaffected
- **Scaling concern:** At 10K+ snippets (documented ceiling), a denser structure would improve cache locality and throughput under heavy load
- **Alternatives:** Sorted vec with binary search for ASCII, HashMap for extended Unicode; Aho-Corasick automaton; or a proper suffix tree

**Action:** Monitor real-world usage; optimize only if benchmarks show it's a bottleneck. Current code is correct and maintainable.

### 2. Buffer and Eviction Trade-offs

**Current:** `max_buffer_chars` defaults to 128 (conservative)

**Interaction:** Large libraries + long triggers + word-boundary mode = complex eviction semantics that correctly fail closed but can surprise users

**Behavior:** When the rolling buffer fills, oldest context is evicted. Word-boundary triggers whose preceding context was evicted cannot match (correct fail-closed behavior). Users with long custom triggers or large libraries may see unexpected non-matches if max_buffer_chars is insufficient.

**Recommendation:** Document the trade-off clearly in the config guide; do not increase default without real-world telemetry. Current conservative default is safer than a generous one that masks misconfiguration.

### 3. Daemon Event Loop Complexity

**Current:** Mixes blocking input sources, 10ms command-completion polls, reconnect logic, window-tracker channel, control socket, and reload signal handling in one loop

**Risk Surface:** Non-trivial under evdev non-exclusive capture + quiet timeout + key-release wait, especially during rapid focus changes or simultaneous typing + window tracking

**Mitigations Already In Place:**
- Generation-aware async command handling (prevents late output from erasing unrelated text)
- Careful reinsert_after logic for non-exclusive capture
- Integration tests for timing-sensitive scenarios

**Assessment:** Well-engineered for its scope, but the surface for subtle timing races is still real. Improvements are incremental (better async abstractions, more targeted tests) rather than architectural overhaul.

### 4. Backend Diversity Complexity

**Current:** Multiple backends (input-method-v2, evdev, libei, wlroots, KWin) mean multiple sites where silent degradation can occur

**Examples:**
- Clipboard X11 fallback on Wayland
- libei keysym synthesis that is layout-dependent
- Protocol edge cases (framing, reconnection, event ordering)

**Assessment:** Project is honest about this in docs and has systematically reduced panics and unbounded operations. Residual risk is more in "Wayland protocol edge cases" than "sloppy engineering". No open code-scanning alerts.

---

## What's Needed to Become a Legendary Product

### Distribution & Discoverability

**Current:** Ubuntu PPA, Arch AUR; Fedora Copr not yet published

**Missing:**
- Official Fedora Copr repository
- Flatpak/Snap with portal support (enables safer deployment in sandboxed environments)
- Simple landing page or demo site (GitHub README is good, but a "Works on my Sway/Hyprland/KDE" gallery helps)
- Short demo video showing first-run experience, snippet creation, and expansion
- "Certified on X compositor version" badges in docs/packaging
- Zero GitHub stars → problem is not code quality but discoverability and proof points

**Action Items:**
- [ ] Publish Fedora Copr (see PACKAGING.md)
- [ ] Invest in simple landing/demo site
- [ ] Create 2-3 short video clips (first-run, snippet creation, real-world use)
- [ ] Build certified-compositor matrix and publish in SUPPORT_MATRIX.md

### Certification on the Big Four Compositors

**Scope:** Sway, Hyprland, KDE Plasma, GNOME

**What's needed:**
- Real end-to-end tests (not just unit tests) on actual display server instances
- Explicit versioning: "wayexpand v1.2 certified on KDE Plasma 6.1.2, Sway 1.9, Hyprland 0.40" (with test date)
- Documented test procedures so users and contributors can verify on new versions
- CI matrix or scheduled job to catch compositor regressions

**Current gaps:**
- wlroots support (Sway, Hyprland, river) is implemented; real-world testing pending
- GNOME window tracking has no protocol path; fallback to app-id-only matching is the ceiling
- Multi-monitor, workspace, focus-stealing scenarios not yet exercised at scale

**Action Items:**
- [ ] Set up E2E test harness for each compositor (could use virtual displays)
- [ ] Run and publish results for v1.2+ release
- [ ] Add to CI or scheduled job (weekly/monthly)

### Resolve the Secure-vs-Fidelity Capture Dilemma

**Current State:** Unresolved; trade-off is documented but forces users to choose between security and usability

**Research Avenues:**
1. **Compositor-provided filtering:** Systemd secure input, input ACLs, or wlroots protocol extension to filter keystrokes by focused window
2. **Better portal/ACL integration:** Improve libei support in compositors; finalize portal token persistence
3. **Input method improvements:** Push for better IME support in GNOME/wlroots so input-method-v2 becomes viable for everyone
4. **Constrained privileged helper:** Not recommended without significant research; complexity and security review burden are high

**Roadmap Impact:** This is a P0 blocker for universal recommendation to desktop Linux users, but does not prevent adoption in controlled environments (sysadmins, trusted machines, single-user sessions).

### IME / Preedit Support (or Excellent Complementary Story)

**Current:** Documented as unsupported; no integration path

**Options:**
1. **Minimal:** Document "finish composition, then expand" workflow clearly; provide hotkey alternative if user can script it
2. **Medium:** Research IBus/Fcitx integration on Linux; coordinate with input-method-v2 work
3. **Ambitious:** Native preedit support in UI (complex; requires toolkit-specific work)

**Action:** Start with documentation and user feedback; only invest in code if demand justifies it.

### Enterprise and Power-User Depth

**Planned:** Action Broker (v1.3+, P1 security gate)

**Also Needed:**
- Shared snippet packs or fleet management (multi-user deployment)
- Audit logging for organizations (who expanded what, when, to where)
- Curated snippet marketplace or git-based sync (lower priority than Action Broker)

### Polish and UX Friction

**Examples:**
- First-run experience: detect compositor, recommend optimal backend with trade-offs explained
- Live "this trigger would expand here" feedback in GUI preview mode
- Snippet search that is instantaneous even with thousands of entries
- More languages (currently English + German)
- Accessibility audit results (target WCAG 2.1 AA)
- Non-Rust-programmer-friendly documentation for extension points

---

## Code Quality Assessment

**Strengths:**
- Modular, clean architecture (engine, backends, frontends are well separated)
- Security-conscious (fail-closed, strict daemon isolation, threat model documented)
- Comprehensive error handling (systematic reduction of panics and unbounded operations)
- Honest about limitations (documentation does not oversell; trade-offs are explicit)
- Well-tested for age and scope (unit tests, workspace tests, integration tests covering edge cases)

**Weaknesses:**
- Performance optimization deferred (fine for current scale; would benefit from benchmarking at 10K+ snippets)
- Daemon event loop complexity (careful but non-trivial surface for timing races)
- Protocol diversity (more backends = more sites for silent degradation, but mitigated by good error handling and docs)

**Overall:** Better foundation than most projects that later become legendary; the remaining work is execution and exposure, not architecture.

---

## Recommended Priority Order

### Immediate (v1.2, current release)
1. Real-world testing of wlroots window tracking (Sway, Hyprland, river)
2. Portal token persistence (libei) — v1.2.1 roadmap item
3. Certification matrix and first-run detection

### Near-term (v1.2+)
4. Action Broker architecture (P1 security gate for enterprise)
5. Fedora Copr publication
6. Landing page + demo video
7. E2E test harness for the big four compositors

### Medium-term (v1.3+)
8. Research and prototype secure-vs-fidelity capture improvements
9. IME research and interim documentation
10. Shared snippet packs / fleet management

### Ongoing
- Reduce friction in first-run experience
- Grow community (contributors, case studies, proof points)
- Maintain stability and backwards-compatibility (COMPATIBILITY.md contract)

---

## References

- [PROFESSIONAL_ROADMAP.md](PROFESSIONAL_ROADMAP.md) — Detailed feature roadmap
- [SECURITY.md](../SECURITY.md) — Threat model and vulnerability disclosure
- [COMPATIBILITY.md](COMPATIBILITY.md) — Stability guarantees
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — Current backend/compositor status
- [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) — Testing procedures
- [ACTION_BROKER_DESIGN.md](ACTION_BROKER_DESIGN.md) — Security gate for v1.3
