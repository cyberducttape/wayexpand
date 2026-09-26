# WayExpand Professional Roadmap

> This roadmap contains historical v1.0/v1.2 planning language as well as
> current open work. For present behavior and certification, use
> `../../SUPPORT_MATRIX.md`, `../../COMPOSITOR_MATRIX.md`, and `wayexpand doctor`.

The core engine, config format, and CLI/JSON contracts are stable as of v1.0.0; desktop backend support is compositor-dependent (see [../../SUPPORT_MATRIX.md](../../SUPPORT_MATRIX.md)). This roadmap covers planned enhancements for 1.x releases and beyond.

## Completed: v1.0.0 (2026-09-17)

Production release with:
- Stability guarantees for CLI, JSON, and config schema
- Security audit and formal threat model documentation
- Package distribution: Ubuntu PPA; Arch packaging is prepared but has not
  been submitted to AUR (Fedora has no Copr repo yet -- see ../../PACKAGING.md)
- Professional GUI with themes, language packs, and accessibility support
- Experimental KDE-specific path (evdev capture + KWin window tracking; not certified)
- Multiple backend coverage (input-method-v2, wlroots, libei/EIS, evdev)
- Stability guarantees documented in COMPATIBILITY.md

## Historical: v1.1.x (2026-09-18)

Shipped as v1.1.0 through v1.1.2. Note this covered accessibility/GUI
polish rather than the wlroots window tracking originally planned for this
slot (moved below to the next unscheduled milestone):
- GUI font scaling (0.8x-2.0x) for accessibility
- 8 color packs, including new Terminal Blue (IBM 3270) and Commodore 64
  retro themes, all meeting WCAG 2.1 AA contrast
- Keyboard focus indicators, typography hierarchy, hover-state polish
- Sysadmin-focused example snippet documentation

---

## Historical v1.2 follow-up

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

### Historical v1.2.1 (Implemented Concurrently)

The following improvements were implemented in v1.2 alongside other work:

- [x] **#25 Command expansions block the input thread** (up to the command
      timeout). Daemon command expansions now run through a bounded background
      queue. Late output is discarded after intervening input, focus, pause,
      window, or reload changes so it cannot erase unrelated text.
- [x] **#27 Command timeouts kill only the direct child.** Spawn commands in
      their own process group and kill the group on timeout.
- [x] **Opt-in portal persistence (#28).** The libei/EIS restoration-token
      flow is implemented with strict local permissions, configurable read/write
      behavior, explicit reset support, and fresh-consent fallback when tokens
      are disabled or invalid. Remaining work is compositor-specific validation.

### Capture Path Improvements (historical v1.2 work)

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

## Historical Prototype: wlroots Window Tracking

The wlroots window-tracker prototype described in earlier planning notes was
removed from the production workspace and is not shipped. `app_filter` window
tracking is currently implemented for KDE only; see
[`docs/BACKENDS.md`](../../BACKENDS.md) and
[`../../SUPPORT_MATRIX.md`](../../SUPPORT_MATRIX.md).

No implementation or certification work for wlroots window tracking should be
inferred from the historical checklist that follows in older revisions.

### Libei-First Backend Auto-Selection (Implemented, Experimental)

**Status:** Implemented (2026-09-19). Conservative automatic selection available.
Production promotion remains gated on compositor/client certification.

**Why:** Reduce user friction by auto-detecting optimal backend
**Scope:** Intelligent backend selection based on compositor detection

**Implementation Details:**
- ✅ Compositor detection (KDE, wlroots, GNOME, X11, Unknown)
- ✅ Libei-first strategy with compositor-specific fallbacks
- ✅ Backend selection respects user overrides
- ✅ Auto-selection only when both flags unset
- ✅ 4 unit tests for auto-select logic
- ✅ Daemon integration with conservative defaults

**Selection Strategy:**
- Daemon automatic selection → conservative stdin + libei; raw evdev and
  input-method-v2 are never enabled implicitly.
- `wayexpand setup` → IBus when the installed component is discoverable and
  policy-permitted; otherwise maximum mode may select evdev + a verified
  libei/wlroots output path.
- Experimental input-method-v2 → explicit opt-in only, with unsupported
  non-text key pass-through visible to the operator.

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

## Planned (v1.3+)

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
unsupported. Completing the broker is a prerequisite for supporting
networked or credentialed admin/SRE command workflows; users must not relax
the shipped daemon unit or treat a wrapper script as a supported escape hatch.

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

See [docs/ACTION_BROKER_DESIGN.md](../../ACTION_BROKER_DESIGN.md) for the
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

**Why:** Current gaps include uncertified compositor support and limited
packaging reach; closing them requires certification, distribution, and
independent proof points.

**Scope:** Close the gap between a strong engineering foundation and a product users trust and recommend.

See [the archived audit findings](docs/archive/2026-09/AUDIT_FINDINGS.md) for historical findings and priority context.

#### Compositor Certification (Sway, Hyprland, KDE Plasma, GNOME)

**Status:** No compositor is certified. KWin window tracking is implemented;
wlroots tracking is not shipped, and GNOME/Mutter has no supported tracker.
Other backend support remains experimental where noted in
[`../../SUPPORT_MATRIX.md`](../../SUPPORT_MATRIX.md).

**What's needed:**
- [ ] Automated end-to-end tests for each compositor covering normal typing, Unicode (emoji, RTL), password fields, focus changes, rapid triggers, undo, and shortcut conflicts
- [ ] Published certified matrix: "wayexpand v1.2 certified on KDE Plasma 6.1.2, Sway 1.9, Hyprland 0.40" with test date
- [ ] First-run experience detects compositor and recommends optimal backend with explicit trade-offs
- [ ] CI matrix or scheduled job to catch regressions on new versions

**Depends on:** wlroots window-tracking real-world testing; GNOME constraints documented in `../../GNOME_WINDOW_TRACKING.md`

#### Distribution & Discoverability

**Current:** Ubuntu PPA is available. Arch packaging is a preview and has not
been submitted to AUR; Fedora Copr is not published.

**What's needed:**
- [ ] Publish Fedora Copr repository (see `../../PACKAGING.md`)
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

**Current:** The IBus engine is implemented and integrated into setup; preedit
and composition support remain future work. Fcitx integration is still research.

---

## Future Considerations (next 1.x milestone)

### Localization Expansion

**Current:** English and German  
**Future:** Community translations via Crowdin or similar  
Community contributions welcome — see CONTRIBUTING.md

### IME & Preedit Support

**Status:** IBus key-event integration exists, but preedit/composition is not supported
**Scope:** Native toolkit integration for composition sequences (ä, é, etc.)  
**Tracker:** INTEGRATION_TESTING.md §Preedit/IME composition

Further work requires compositor- and toolkit-specific testing.

### Performance Optimization

**Baseline established:** See ../../GUI.md
Future work: reduce daemon cold-start latency, optimize matcher for 10K+ snippets

### Release Architectures

The GitHub release workflow publishes a prebuilt Linux archive for x86_64
only. There is no official aarch64 release archive yet; aarch64 users build
from source. Arch packaging is prepared but has not been submitted to AUR.
See [../../PACKAGING.md](../../PACKAGING.md) for current details.

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
- ✅ Available via Ubuntu PPA
- ◐ Arch PKGBUILD prepared; AUR publication is pending
- ✅ Documented security policy with vulnerability disclosure process
- ✅ Stability guarantees (../../COMPATIBILITY.md)
- Historical v1.0 milestone criterion; this is not a current defect-status
  assertion. See the current support matrix and regression tests for status.
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
**Historical v1.1.x:** Released 2026-09-18 (GUI accessibility, themes, docs)
**Current 1.2.x line:** No compositor is certified by the automated evidence
matrix yet. The next milestone prioritizes a golden KDE/KWin certification,
the shared deferred pipeline for every live capture backend, and measured
follow-up work such as wlroots window tracking.

---

## Contributing to the Roadmap

Have an idea for a future 1.x release? Open an issue on GitHub or submit a
pull request. See CONTRIBUTING.md for guidelines.

Priority goes to:
1. Bug fixes and security patches (any version)
2. Compositor compatibility and certification evidence
3. Community-requested features for future 1.x releases
4. Performance and localization enhancements

---

## References

- [COMPATIBILITY.md](../../COMPATIBILITY.md) — Stability guarantees and migration policy
- [SECURITY.md](../../SECURITY.md) — Threat model and vulnerability disclosure
- [CONTRIBUTING.md](../../CONTRIBUTING.md) — How to contribute code and ideas
- [SUPPORT_MATRIX.md](../../SUPPORT_MATRIX.md) — Current backend status by compositor
- [INTEGRATION_TESTING.md](../../INTEGRATION_TESTING.md) — Testing and certification process
