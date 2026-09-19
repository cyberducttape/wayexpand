# WayExpand Professional Roadmap

The core engine, config format, and CLI/JSON contracts are stable as of v1.0.0; desktop backend support is compositor-dependent (see [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md)). This roadmap covers planned enhancements for 1.x releases and beyond.

## Completed: v1.0.0 (2026-09-17)

Production release with:
- Stability guarantees for CLI, JSON, and config schema
- Security audit and formal threat model documentation
- Package distribution: Ubuntu PPA and Arch AUR (Fedora has no Copr repo
  yet -- see docs/PACKAGING.md)
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

## Before v1.2

Open items that hold the v1.2 tag. (#15, doctor recognizing evdev+libei, and
#19, app_filter preferring `app_id` over window title, are done.)

- [ ] **#16 Mark v1.1.2 as the latest GitHub release.** GitHub's
      `/releases/latest` still resolves to v0.2.1. Manual: Releases → v1.1.2 →
      Edit → "Set as the latest release".
- [x] **#17 Separate implementation status from environment status in
      diagnostics.** `BackendState` mixes "not implemented" with "needs
      permission" (uinput reports `RequiresPermission` although no uinput
      backend exists). Report implementation, device/protocol presence,
      permission and connection separately, in `doctor` and `doctor --json`.
- [x] **#18 Contract test for documented backend states.** COMPATIBILITY.md
      now lists the real `BackendState` values; add a test that fails when the
      enum and the documented list diverge. Depends on #17.
- [x] **#20 App context for previewing `app_filter` snippets.** Preview has
      no focused window, so app-restricted snippets always show "no match".
      Add an app selector (GUI, TUI) and `--preview-app=<id>` (CLI) that feeds
      a simulated `WindowChanged`. Expected: `app_filter = ["thunderbird"]`
      matches with `thunderbird` selected, not with `konsole` or no context.
- [x] **evdev: text typed to end a trigger is erased instead of the
      trigger's first character.** evdev capture is non-exclusive, so the
      space/Enter/other key that completes a word-boundary trigger has already
      reached the app. Fixed: the engine now reports `reinsert_after`, and
      backends erase both trigger and terminator, insert replacement, then
      re-insert the terminator. Corrects `:sig ` → `signature ` instead of
      `:regards ` (missing space).

### v1.2.1

- [x] **#25 Command expansions block the input thread** (up to the command
      timeout). Daemon command expansions now run through a bounded background
      queue. Late output is discarded after intervening input, focus, pause,
      window, or reload changes so it cannot erase unrelated text.
- [x] **#27 Command timeouts kill only the direct child.** Spawn commands in
      their own process group and kill the group on timeout.
- [ ] **Opt-in portal persistence (#28).** Offer an explicit, revocable persistence
      flow for libei/EIS restoration tokens, stored with strict permissions;
      keep the current non-persistent consent behavior as the default.
      **Implementation guide:**
      - Current: `select_devices()` uses `PersistMode::DoNot` (line 731)
      - Proposed: Change to `PersistMode::Persistent` and extract restoration token from session
      - Storage: `~/.config/wayexpand/libei-portal-token` (mode 0600, user-only)
      - Config: Add `settings.libei_token_persistence` (bool, default: true) in config.rs
      - Restoration: On next connection, call `proxy.restore_session(token)` instead of `create_session()`
      - Error recovery: If restoration fails (expired/invalid), fall back to fresh `create_session()`
      - Testing: Verify consent dialog appears once on first run, disappears on reconnect if token valid
      - Note: ashpd version in vendor/ may need API verification; check Session type in ashpd::desktop

### Capture Path Improvements (v1.2.1+)

**Known tradeoff:** No capture mode is currently both secure and universal.

- **input-method-v2:** Protects password fields and respects sensitive signals,
  but exclusive keyboard grab cannot safely pass through Escape, arrow keys,
  and function keys. Users on GNOME report losing navigation capability.
- **evdev:** Preserves all keyboard events and now mirrors kernel repeat into
  matcher state, but reads all global keystrokes (requires `input` group) and
  ignores password field signals. Repeat still needs real-device certification.

Future work: research platform-specific approaches (systemd secure input,
input ACLs, or compositor-provided focused-window filtering) to recover both
security and fidelity in one path. This blocks universal recommendation to
desktop Linux users but does not prevent use in controlled environments
(sysadmins, trusted machines, single-user sessions).

---

## In Progress (v1.2 target)

### Window Tracking for wlroots Compositors

**Status:** Phases 1-2 complete (2026-09-19). Phase 3 (daemon integration) in progress.
**Why:** Complete `app_filter` support across Sway, Hyprland, river  
**Scope:** Implement wlroots `wlr-foreign-toplevel-management-v1` protocol

**Phase 1 + 2 Complete:**
- ✅ Protocol connection and registry discovery
- ✅ Toplevel event handling (creation, destruction, metadata)
- ✅ Focus tracking with async notifications
- ✅ Channel-based communication for non-blocking window changes
- ✅ WindowTracker trait implementation with timeout support
- ✅ 3 new tests covering protocol detection, naming, timeout behavior

**Phase 3 (TODO - ~1 hour):**
- [ ] Integrate tracker into daemon event loop
- [ ] Add backend discovery function for availability detection
- [ ] Route window focus changes to engine.process(InputEvent::WindowChanged)
- [ ] Wire up app_filter matching with focused window context

**Commits:**
- 1f58bc43: Phase 1 foundation
- 0f48b022: Phase 2 event-driven focus tracking

**Status by compositor:**
- Sway: foundation ready, needs Phase 3 + real-world testing
- Hyprland: foundation ready, needs Phase 3 + real-world testing
- river: foundation ready, needs Phase 3 + real-world testing

**Related:**
- `crates/backend-wlroots-toplevel/src/lib.rs`: full Phase 1-2 implementation
- `docs/WLROOTS_WINDOW_TRACKING_GUIDE.md`: implementation details
- `docs/DESKTOP_STATUS.md`: current status by compositor
- GNOME/Mutter: no window-tracking protocol exists (see GNOME_WINDOW_TRACKING.md)

### Polish & Quality Improvements

- [x] ~~Remove emoji and uncommon Unicode glyphs from functional GUI controls~~
      -- controls now use text labels; decorative illustrations remain optional
      and do not carry functionality
- [x] ~~Move long-running KWin window-tracker operations (used by "Use
      current app" in the snippet editor) to a background thread~~ -- done:
      detection runs on a worker thread and the GUI polls for the result
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

### Release Architectures

Native x86_64 and aarch64 release archives are built on GitHub-hosted Linux
runners. Package-manager publication and installation testing on aarch64 remain
distribution-maintainer verification tasks.

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
  see [docs/PACKAGING.md](docs/PACKAGING.md))
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
