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

## v1.2 follow-up

Open items that hold the v1.2 tag. (#15, doctor recognizing evdev+libei, and
#19, app_filter preferring `app_id` over window title, are done.)

- [x] **#16 Promote the current release on GitHub.** The old v1.1.2 action was
      superseded when v1.2 was published; `/releases/latest` now resolves to
      the newer release. Repeat the same UI check after future releases.
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

**Status:** Phases 1-3 complete (2026-09-19). Ready for real-world testing.
**Why:** Complete `app_filter` support across Sway, Hyprland, river  
**Scope:** Implement wlroots `wlr-foreign-toplevel-management-v1` protocol

**Phase 1 + 2 + 3 Complete:**
- ✅ Protocol connection and registry discovery
- ✅ Toplevel event handling (creation, destruction, metadata)
- ✅ Focus tracking with async notifications
- ✅ Channel-based communication for non-blocking window changes
- ✅ WindowTracker trait implementation with timeout support
- ✅ Daemon event loop integration
- ✅ Window focus changes routed to engine.process(InputEvent::WindowChanged)
- ✅ app_filter matching with focused window context
- ✅ Workspace tests covering protocol detection, naming, timeout, daemon integration,
  configuration safety, matching, and backend behavior; run
  `cargo test --locked --workspace` for the current count

**Commits:**
- 1f58bc43: Phase 1 foundation
- 0f48b022: Phase 2 event-driven focus tracking
- 97b49438: Phase 3 daemon integration

**Status by compositor:**
- Sway: ✅ Ready (Phase 3 complete, needs real-world testing)
- Hyprland: ✅ Ready (Phase 3 complete, needs real-world testing)
- river: ✅ Ready (Phase 3 complete, needs real-world testing)
- GNOME: ❌ No usable protocol path (see `docs/archive/GNOME_WINDOW_TRACKING.md`)

### Libei-First Backend Auto-Selection (Phase 4)

**Status:** Complete (2026-09-19). Ready for production use.
**Why:** Reduce user friction by auto-detecting optimal backend
**Scope:** Intelligent backend selection based on compositor detection

**Implementation Complete:**
- ✅ Compositor detection (KDE, wlroots, GNOME, X11, Unknown)
- ✅ Libei-first strategy with compositor-specific fallbacks
- ✅ Backend selection respects user overrides
- ✅ Auto-selection only when both flags unset
- ✅ 4 unit tests for auto-select logic
- ✅ Daemon integration (Phase 4 UX improvement)

**Selection Strategy:**
- KDE Plasma → input-method-v2
- Sway/Hyprland/river → evdev + libei (libei-first)
- GNOME → input-method-v2
- X11 → evdev
- Unknown → stdin + libei

**Commits:**
- e340be2c: Phase 4 daemon integration
- a54913d2: Optimistic state tracking for input-method

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

## In Progress / Planned (v1.3+)

### P1 Security Gate: Action Broker

**Status:** P1 architecture gate; not implemented
**Why:** Current command-backed expansions run in the hardened daemon sandbox,
which intentionally prevents legitimate SRE tools (`kubectl`, `aws`, `vault`,
`ssh`, `terraform`, and similar workflows). This is a product boundary, not a
reason to weaken the capture daemon.
**Scope:** Separate capture/match/inject daemon from an optional policy-driven
action broker

Until this gate is complete, WayExpand supports direct commands only for
sandbox-compatible local actions. Infrastructure actions are explicitly
unsupported; users must not relax the shipped daemon unit or treat a wrapper
script as a supported escape hatch.

**Design:**
- Capture process stays extremely locked down (no network, no HOME write, strict syscalls)
- Action broker runs with exactly the permissions needed for each environment
- Per-expansion policy: allowed commands, network access, filesystem paths, environment variables
- Organization can set `safe_mode = true` to completely disable command execution
- Commands are whitelisted by name, not arbitrary executables

**Benefit:** Enables enterprise/SRE use cases without weakening the capture daemon's security posture

**Exit criteria:**
- [ ] Protocol is bounded, versioned, authenticated with peer credentials, and
      action-name based; the daemon cannot submit arbitrary executable paths,
      arguments, or environment variables.
- [ ] Broker has an independently hardened systemd unit and trusted action
      configuration ownership checks.
- [ ] Per-action executable, arguments, environment, cwd, network policy,
      timeout, output limit, and audit result are enforced by the broker.
- [ ] Broker denial/unavailability fails closed; the daemon never falls back to
      local execution.
- [ ] Integration tests cover framing, auth, policy denial, timeout, output
      limits, and the absence of daemon network/home access.

See [docs/ACTION_BROKER_DESIGN.md](docs/ACTION_BROKER_DESIGN.md) for the
protocol, threat boundary, and acceptance criteria.

**Architecture:**
```
Daemon (capture/match/inject)
   ├─ ProtectHome=strict
   ├─ ProtectSystem=strict
   ├─ RestrictAddressFamilies=AF_UNIX
   └─ No network/FS access
      │
      └─ constrained IPC (command name + args only)
         ▼
      Optional Action Broker (per-user or per-org policy)
         ├─ Whitelist enforcement
         ├─ Network policy
         ├─ FS sandbox (per-command)
         └─ Resource limits
```

---

### Path to Legendary Status: Certification and Distribution

**Why:** Current gaps (zero GitHub stars, unverified compositor support, limited packaging reach)
are not due to code quality but execution: certification, distribution, and proof points.

**Scope:** Close the gap between a strong engineering foundation and a product users trust and recommend.

See [docs/AUDIT_FINDINGS.md](docs/AUDIT_FINDINGS.md) for detailed findings and recommended priority order.

#### Compositor Certification (Sway, Hyprland, KDE Plasma, GNOME)

**Status:** Window tracking and backend support are implemented; real-world validation pending

**What's needed:**
- [ ] Automated end-to-end tests for each compositor covering normal typing, Unicode (emoji, RTL), password fields, focus changes, rapid triggers, undo, and shortcut conflicts
- [ ] Published certified matrix: "wayexpand v1.2 certified on KDE Plasma 6.1.2, Sway 1.9, Hyprland 0.40" with test date
- [ ] First-run experience detects compositor and recommends optimal backend with explicit trade-offs
- [ ] CI matrix or scheduled job to catch regressions on new versions

**Depends on:** wlroots window-tracking real-world testing; GNOME constraints documented in `docs/GNOME_WINDOW_TRACKING.md`

#### Distribution & Discoverability

**Current:** Ubuntu PPA, Arch AUR; Fedora Copr not yet published

**What's needed:**
- [ ] Publish Fedora Copr repository (see `docs/PACKAGING.md`)
- [ ] Flatpak/Snap builds with portal support
- [ ] Simple landing/demo site (GitHub README is good; visual proof points help)
- [ ] 2-3 short demo videos: first-run, snippet creation, real-world use
- [ ] README badges for certified compositor + packaging status

#### Resolve Secure-vs-Fidelity Capture Trade-off

**Current:** Documented but forces users to choose between security and usability.

**Research needed:**
- Compositor-provided focused-window input filtering (systemd secure input, input ACLs)
- Improved input-method protocol in GNOME/wlroots
- Portal/ACL improvements for libei
- (Not recommended: constrained privileged helper — security review burden is high)

**Blocker for:** Universal recommendation to desktop Linux users; does not prevent adoption in controlled environments (sysadmins, trusted machines)

#### IME / Preedit Support

**Current:** Explicitly unsupported; documented limitation

**Interim approach:**
- [ ] Clear documentation of "finish composition, then expand" workflow
- [ ] Optional hotkey for expanding after composition
- [ ] Accessibility audit for RTL input

**Future:** Research IBus/Fcitx integration and coordinate with input-method-v2 improvements

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

### Professional Tier (Achieved v1.0.0)
- ✅ Available via Ubuntu PPA and Arch AUR
- ✅ Documented security policy with vulnerability disclosure process
- ✅ Stability guarantees (COMPATIBILITY.md)
- ✅ No known critical bugs
- ✅ Public changelog for releases
- ✅ GitHub repository with active CI

### Legendary Tier (In Progress)
Requires all of the above, plus:
- **Certification:** Documented end-to-end test results for Sway, Hyprland, KDE Plasma, and GNOME (exact versions, test dates, known limitations)
- **Distribution:** Fedora Copr, Flatpak/Snap with portal support, and simple landing page with demo video
- **Proof:** 1000+ GitHub stars, 5+ active contributors (beyond original author), documented corporate/organization deployments
- **Capture clarity:** Resolved or well-researched path to secure-vs-fidelity trade-off (research direction published, if not yet solved)
- **Accessibility:** WCAG 2.1 AA audit results; localization to 3+ languages
- **IME story:** Either native support or clear interim workflow with documentation

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
