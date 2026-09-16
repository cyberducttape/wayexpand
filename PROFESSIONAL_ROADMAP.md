# WayExpand Professional Maturity Roadmap

This document tracks work to elevate WayExpand from a solid technical project to a professional, production-ready tool that enterprises and end-users can trust.

## ✅ Completed (This Session)

### 1. CI Robustness (Priority: Critical)
- **Fixed:** `test-install-user.sh` now handles environments where `TMPDIR` is not set
- **Added:** CI debugging output to diagnose environment issues
- **Result:** Script is more resilient to GitHub Actions runner quirks

### 2. Release Process & Versioning (Priority: Critical)
- **Bumped:** Version from 0.1.0 → 0.2.0 in `Cargo.toml`
- **Created:** Release notes in CHANGELOG dated 2026-09-16
- **Documented:** Release process in this roadmap
- **Status:** Ready to tag `v0.2.0` and create GitHub release

### 3. Contributing Guidelines (Priority: Important)
- **Wrote:** Comprehensive `CONTRIBUTING.md` with:
  - Quick-start setup for developers
  - Pre-PR checklist (same as CI)
  - Project architecture overview
  - Common development tasks
  - Security and bug reporting guidance
- **Linked:** References to SECURITY.md and detailed wiki guides

### 4. Dependency Auditing (Priority: Important)
- **Status:** ✅ Already implemented via `actions-rust-lang/audit@v1` in CI
- **Runs:** Every push, checks for known vulnerabilities in dependencies

---

## 📋 Remaining Work (Prioritized)

### High Priority (Blocks Professional Adoption)

#### 1. Package Distribution
**Status:** Not started  
**Effort:** ~2-4 hours per package manager  
**Why:** Users should install from their distro's package manager, not build from source

**Tasks:**
- [ ] Create AUR (Arch Linux) package (via `PKGBUILD` in repo)
- [ ] Set up Debian/Ubuntu PPA (consider using `ppa:itchyitchy123/wayexpand`)
- [ ] Create Fedora/RPM spec file
- [ ] Document installation methods in README

**Implementation notes:**
- AUR: Fork-and-submit process with `PKGBUILD` template
- Debian PPA: Use Launchpad for automated builds on each git tag
- Fedora: Spec file in repo, submit to Fedora package collection

#### 2. Security Policy (Enhance Existing)
**Status:** `SECURITY.md` exists but is minimal  
**Effort:** ~1 hour  
**Why:** Professional projects need clear vulnerability disclosure process

**Current content in SECURITY.md:**
- Basic threat model
- Sensitive field handling
- Logging policy

**To add:**
- [ ] Vulnerability disclosure timeline (48-hour embargo for fixes)
- [ ] Security contacts and PGP key for encrypted reports
- [ ] Known limitations and CVE tracking
- [ ] Incident response process
- [ ] Cryptographic standards for any password/token handling (if added later)

#### 3. Accessibility Audit & Fixes
**Status:** Not started  
**Effort:** ~4-6 hours  
**Why:** GUI should be usable by people with disabilities; improves user base

**Tasks:**
- [ ] Audit GUI keyboard navigation:
  - [ ] Tab order in snippet editor
  - [ ] Arrow keys for list navigation
  - [ ] Enter to edit/delete
  - [ ] Escape to cancel
- [ ] Test screen reader compatibility (NVDA/JAWS on Windows, Orca on Linux)
- [ ] Verify color contrast (WCAG AA minimum)
- [ ] Document accessibility features in README

**Implementation:**
- Use egui's built-in a11y support
- Test with actual assistive technology
- Add alt-text for any diagrams in docs

### Medium Priority (Improves User Experience)

#### 4. API Documentation
**Status:** Library API (`wayexpand-core`) not documented  
**Effort:** ~2 hours  
**Why:** Users should be able to embed WayExpand expansion engine in their tools

**Tasks:**
- [ ] Add doc comments to public functions in `crates/core/src/`
- [ ] Run `cargo doc --open` and verify renders cleanly
- [ ] Add examples in function doc comments
- [ ] Document engine initialization and event loop

#### 5. Performance Benchmarks
**Status:** Not started  
**Effort:** ~2 hours  
**Why:** Establish baseline; prove expansion latency is imperceptible

**Tasks:**
- [ ] Benchmark expansion latency: trigger → output visible (~50ms target)
- [ ] Benchmark memory usage: daemon RSS under typical load
- [ ] Benchmark CPU: idle + during expansion
- [ ] Document results in `docs/PERFORMANCE.md`

#### 6. Integration Tests
**Status:** Rust unit tests exist; no end-to-end tests  
**Effort:** ~3 hours  
**Why:** Daemon + GUI + backends should work together

**Tasks:**
- [ ] Create `tests/integration/` directory
- [ ] Write test: start daemon, expand snippet via CLI, verify output
- [ ] Test: GUI creates snippet, daemon applies it, result correct
- [ ] Test: all backends (input-method-v2, libei, wlroots, evdev) with dummy expansion

### Lower Priority (Polish & Future Growth)

#### 7. Internationalization (i18n)
**Status:** English only  
**Effort:** ~4-6 hours (framework) + community translations  
**Why:** Global accessibility

**Approach:**
- Use `fluent` crate for message catalogs
- Extract strings to `.ftl` format
- Set up Crowdin or similar for community translations
- Test with at least one additional language (Spanish, German, Japanese)

#### 8. Telemetry (Privacy-Respecting)
**Status:** None  
**Effort:** ~3 hours  
**Why:** Understand which features are used, what bugs affect real users

**Constraints:**
- No user identification
- No expansion content ever
- Opt-in (flag in config, default disabled)
- Metrics: which backends active, which features used, crash counts
- Local-only aggregation (no cloud; users control their data)

#### 9. Community Channels
**Status:** Only GitHub Issues  
**Effort:** ~30 minutes setup  
**Why:** Users prefer chat for quick questions

**Options:**
- Matrix room: `#wayexpand:matrix.org` (lightweight, no cost)
- Discord: lighter community feel, better for casual discussion
- Neither is necessary if GitHub Discussions gain traction

#### 10. FAQ & Troubleshooting Guide
**Status:** `docs/SUPPORT_MATRIX.md` exists but is technical  
**Effort:** ~1.5 hours  
**Why:** Users Google "why doesn't my expansion work"

**Sections:**
- [ ] "Expansion not firing" → check triggers, match-mode, buffer size
- [ ] "UI hangs when opening Settings" → known issue on slow systems; workaround
- [ ] "Characters drop in [GNOME|KDE|Sway]" → which backend, known limitations
- [ ] Performance tips: minimize buffer, use immediate matching
- [ ] Backup/restore configuration

---

## Release Schedule Suggestion

### v0.2.0 (Current — Ready to Release)
- ✅ Evdev backend fully functional
- ✅ GUI polish complete
- ✅ Character-drop bug fixed
- ✅ All tests passing
- **Action:** Tag `v0.2.0`, create GitHub release with assets

### v0.3.0 (Next Major Update — 1 month)
- Package distribution (AUR, Debian, Fedora)
- Accessibility audit + fixes
- Performance benchmarks documented
- Integration tests

### v1.0.0 (Stable Release — 2-3 months)
- Reaches this milestone when:
  - Packages available in all major distros
  - Security policy fully documented and tested
  - No known critical bugs
  - Community feedback incorporated
  - API stable (no breaking changes between 0.3 → 1.0)

---

## Success Criteria for Professional Status

- [ ] Available in 3+ package managers (AUR, Debian, Fedora)
- [ ] 1000+ GitHub stars
- [ ] Documented security policy with vulnerability disclosure process
- [ ] Passes accessibility audit (WCAG AA)
- [ ] Performance benchmarks published
- [ ] Integration test suite complete
- [ ] Public changelog for every release
- [ ] 5+ active contributors (beyond original author)
- [ ] Corporate/organization deployments documented (even anonymized)

---

## Next Immediate Action

1. **Test the 0.2.0 release locally:**
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

2. **GitHub will auto-build and create a release** via the release workflow

3. **Start on package distribution** (highest ROI for user adoption):
   - Begin with AUR (easiest)
   - Then Debian PPA
   - Then Fedora
