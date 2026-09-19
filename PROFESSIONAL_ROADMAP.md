# WayExpand Professional Roadmap

The core engine, config format, and CLI/JSON contracts are stable as of v1.0.0; desktop backend support is compositor-dependent (see [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md)). This roadmap covers planned enhancements for 1.x releases and beyond.

## Completed: v1.0.0 (2026-09-17)

Production release with:
- Stability guarantees for CLI, JSON, and config schema
- Security audit and formal threat model documentation
- Package distribution: Ubuntu PPA and Arch AUR (Fedora has no Copr repo
  yet -- see docs/PACKAGING_CHECKLIST.md)
- Professional GUI with themes, language packs, and accessibility support
- KDE Plasma support (evdev capture + KWin window tracking)
- Multiple backend coverage (input-method-v2, wlroots, libei/EIS, evdev)
- Stability guarantees documented in COMPATIBILITY.md

## Completed: v1.1.x (2026-09-18)

Shipped as v1.1.0 through v1.1.2. Note this covered accessibility/GUI
polish rather than the wlroots window tracking originally planned for this
slot (moved below to the next unscheduled milestone):
- GUI font scaling (0.8x-2.0x) for accessibility
- 8 color packs, including new Terminal Blue (IBM 3270) and Commodore 64
  retro themes, all meeting WCAG 2.1 AA contrast
- Keyboard focus indicators, typography hierarchy, hover-state polish
- Sysadmin-focused example snippet documentation

---

## Next (unscheduled)

### Window Tracking for wlroots Compositors

**Status:** Not started -- no committed release, moved here from the
original v1.1 slot since v1.1.x shipped other work instead  
**Why:** Complete `app_filter` support across all major compositors  
**Scope:** Implement wlroots `wlr-foreign-toplevel-management-unstable-v1` protocol for Sway/Hyprland/river

- [ ] Implement wlroots toplevel tracker backend
- [ ] Add integration tests for Sway and Hyprland
- [ ] Update SUPPORT_MATRIX.md to mark wlroots as Supported
- [ ] Add troubleshooting guide for app-filter usage

**Related issues:**
- RELEASE_1.0_CHECKLIST.md §5 mentions protocol is standard and lower risk
- Currently implemented only for KDE Plasma (D-Bus)
- GNOME/Mutter: no window-tracking implementation exists and none is
  scheduled. (This roadmap previously called it "out of scope" while
  SUPPORT_MATRIX.md called it "planned for v1.1" -- neither was accurate;
  this is the corrected, single source of truth for its status.)

### Polish & Quality Improvements

- [ ] Move long-running KWin window-tracker operations (used by "Use
      current app" in the snippet editor) to a background thread -- it
      currently blocks the GUI thread for up to 5 seconds
- [x] ~~Cleanup predictable `/tmp/wayexpand-window-tracker-*.js` files on
      daemon startup~~ -- done: the path now includes a random component
      and is opened with `O_CREAT|O_EXCL`, refusing to write through
      anything already present (see CHANGELOG.md, Security fixes)

---

## Future Considerations (v1.2+)

### Localization Expansion

**Current:** English and German  
**Future:** Community translations via Crowdin or similar  
Community contributions welcome — see CONTRIBUTING.md

### IME & Preedit Support

**Status:** Known limitation, not supported  
**Scope:** Native toolkit integration for composition sequences (ä, é, etc.)  
**Tracker:** INTEGRATION_TESTING.md §Preedit/IME composition

Research ongoing; requires compositor-specific testing.

### Performance Optimization

**Baseline established:** See docs/GUI_PERFORMANCE.md  
Future work: reduce daemon cold-start latency, optimize matcher for 10K+ snippets

### Enhanced Telemetry (Privacy-Respecting)

**Constraints:**
- Opt-in only (default disabled, flag in config)
- No user identification or snippet content
- Local aggregation (no cloud)
- User control over what's shared

**Proposed metrics:** which backends active, features used, crash counts

---

## Success Metrics for Professional Status

Achieved at v1.0.0:
- Available via Ubuntu PPA and Arch AUR (Fedora Copr not yet published —
  see [docs/PACKAGING_CHECKLIST.md](docs/PACKAGING_CHECKLIST.md))
- Documented security policy with vulnerability disclosure process
- Stability guarantees (COMPATIBILITY.md)
- No known critical bugs
- Public changelog for releases
- GitHub repository with active CI

Ongoing:
- 1000+ GitHub stars (community adoption)
- 5+ active contributors (beyond original author)
- Corporate/organization deployments (documented)
- Accessibility audit results (WCAG AA target)

---

## Release Schedule

**v1.0.0:** Released 2026-09-17  
**v1.1.x:** Released 2026-09-18 (GUI accessibility, themes, docs)  
**v1.2+:** No committed date. wlroots window tracking is the leading
candidate; otherwise driven by community feedback and contributions.

---

## Contributing to the Roadmap

Have an idea for v1.1+? Open an issue on GitHub or submit a pull request. See CONTRIBUTING.md for guidelines.

Priority goes to:
1. Bug fixes and security patches (any version)
2. Compositor compatibility improvements (v1.1 focus)
3. Community-requested features (v1.2+)
4. Performance and localization enhancements

---

## References

- [COMPATIBILITY.md](docs/COMPATIBILITY.md) — Stability guarantees and migration policy
- [SECURITY.md](SECURITY.md) — Threat model and vulnerability disclosure
- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute code and ideas
- [SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) — Current backend status by compositor
- [INTEGRATION_TESTING.md](docs/INTEGRATION_TESTING.md) — Testing and certification process
