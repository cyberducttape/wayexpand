# WayExpand Professional Roadmap

WayExpand v1.0.0 is production-ready and recommended for deployment on Wayland desktops. This roadmap covers planned enhancements for 1.x releases and beyond.

## ✅ Completed: v1.0.0 (2026-09-17)

Production release with:
- ✅ Stability guarantees for CLI, JSON, and config schema
- ✅ Security audit and formal threat model documentation
- ✅ Comprehensive package distribution (Ubuntu PPA, Arch AUR, Fedora Copr)
- ✅ Professional GUI with themes, language packs, and accessibility support
- ✅ Full KDE Plasma support (evdev capture + KWin window tracking)
- ✅ Multiple backend coverage (input-method-v2, wlroots, libei/EIS, evdev)
- ✅ Stability guarantees documented in COMPATIBILITY.md

---

## 🚀 Planned: v1.1 (Next Release)

### Window Tracking for wlroots Compositors

**Status:** Planned for 1.1  
**Why:** Complete `app_filter` support across all major compositors  
**Scope:** Implement wlroots `wlr-foreign-toplevel-management-unstable-v1` protocol for Sway/Hyprland/river

- [ ] Implement wlroots toplevel tracker backend
- [ ] Add integration tests for Sway and Hyprland
- [ ] Update SUPPORT_MATRIX.md to mark wlroots as Supported
- [ ] Add troubleshooting guide for app-filter usage

**Related issues:**
- RELEASE_1.0_CHECKLIST.md §5 mentions protocol is standard and lower risk
- Currently implemented only for KDE Plasma (D-Bus)
- GNOME/Mutter has no equivalent (shell-extension required, out of scope)

### Polish & Quality Improvements

- [ ] Move long-running KWin window-tracker operations to background thread (reduce UI jank)
- [ ] Cleanup predictable `/tmp/wayexpand-window-tracker-*.js` files on daemon startup
- [ ] Document cleanup story in SECURITY.md or release notes

---

## 💡 Future Considerations (v1.2+)

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

## 🎯 Success Metrics for Professional Status

Achieved at v1.0.0:
- ✅ Available in 3+ package managers (AUR, Debian, Fedora)
- ✅ Documented security policy with vulnerability disclosure process
- ✅ Stability guarantees (COMPATIBILITY.md)
- ✅ No known critical bugs
- ✅ Public changelog for releases
- ✅ GitHub repository with active CI

Ongoing:
- 1000+ GitHub stars (community adoption)
- 5+ active contributors (beyond original author)
- Corporate/organization deployments (documented)
- Accessibility audit results (WCAG AA target)

---

## Release Schedule

**v1.0.0:** Released 2026-09-17 ✅  
**v1.1:** Planned Q4 2026 (wlroots window tracking, polish)  
**v1.2+:** Features driven by community feedback and contributions

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
