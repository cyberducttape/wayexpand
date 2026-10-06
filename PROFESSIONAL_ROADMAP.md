# WayExpand Professional Roadmap

> This is the forward-looking roadmap. Completed work and historical release
> planning are archived in [docs/archive/2026-09/PROFESSIONAL_ROADMAP.md](docs/archive/2026-09/PROFESSIONAL_ROADMAP.md);
> current behavior is defined by the support and compatibility documents.

As of v1.3.3, the core engine, configuration format, and CLI/JSON contracts
are stable. Desktop backend support remains experimental until the
certification evidence described in [docs/CERTIFICATION.md](docs/CERTIFICATION.md)
and [docs/SUPPORT_MATRIX.md](docs/SUPPORT_MATRIX.md) is published.

## Production blockers

These are the work items that most directly determine whether WayExpand is
safe to recommend for everyday desktop use.

### 1. KDE/KWin certification

- [ ] Run the complete end-to-end matrix on a supported KDE Plasma/KWin version.
- [ ] Cover Firefox, Chromium, Qt, GTK, Electron, terminals, Unicode, rapid
      typing, shortcuts, focus changes, layout changes, restart, suspend/resume,
      portal revocation, and device reconnect.
- [ ] Publish exact versions, backend choices, test dates, and known limitations.

### 2. GNOME certification

- [ ] Establish the supported GNOME/Mutter capture and injection path.
- [ ] Test GTK, Electron, Firefox, Chromium, terminals, password fields, focus
      changes, and compositor/session restart.
- [ ] Document whether the result is certified, experimental, or unsupported.

### 3. Sway and Hyprland certification

- [ ] Certify the applicable input-method-v2, libei, and wlroots paths on
      representative versions.
- [ ] Test text controls, terminals, Unicode, rapid input, shortcuts, focus
      changes, and reconnect behavior.
- [ ] Record compositor-specific limitations in the support matrix.

### 4. Keyboard-layout and input correctness

- [ ] Validate US, German, French, AltGr, dead keys, Compose, non-Latin
      layouts, multiple configured layouts, and runtime layout switching.
- [ ] Exercise two simultaneous keyboards, USB disconnect/reconnect, held keys,
      suspend/resume, and rapid typing.
- [ ] Treat layout or keymap mismatches as certification failures.
- [ ] libei keysym fallback: verify on KWin and other EIS servers that
      `ei_keyboard.modifiers` reports layout-group and Caps Lock changes and
      that a keymap change replaces the device. The backend now follows those
      reports, refuses multi-layout keymaps until the active layout is known,
      and reconnects when its device is paused or removed.

### 5. Certification-driven route planner

`recommended_route()` only knows IBus and ignores the capability probe, so a
machine with a better route still reports no safe automatic path.

- [ ] Discover routes, negotiate capabilities, load local certification
      evidence, apply organization policy, then rank: certified native
      text-input, certified IBus/Fcitx, certified libei, experimental routes,
      and evdev only with explicit consent.
- [ ] Label each route as recommended, experimental, or requiring consent in
      setup, the GUI, and `doctor`. Evdev must never be chosen without consent.

### 6. IME and preedit strategy

Preedit/IME composition is currently unsupported. Decide and document the
supported end state before broad adoption:

- [ ] Evaluate native text-input/IME integration for composition-aware
      expansion.
- [ ] Define an explicit Fcitx/Rime/IBus strategy. A native Fcitx5 addon and
      deeper IBus engine integration are likely cleaner than inferring
      composition state from raw key streams; scope this as a 1.5/2.0 project
      and position earlier releases explicitly for direct/committed text.
- [ ] Certify Chinese, Japanese, Korean, dead-key, and Compose workflows or
      document their supported fallback behavior.
- [ ] Keep the limitation prominent in docs/SUPPORT_MATRIX.md until evidence
      exists.

### 7. Crash, restart, and lifecycle soak testing

- [ ] Run long-duration typing tests across compositor restart, daemon restart,
      portal revocation, suspend/resume, focus changes, reloads, and device
      reconnects.
- [ ] Verify no input loss, stale expansion, unsafe replacement, stuck key,
      or restart-loop behavior.
- [ ] Publish representative soak duration and results.
- [x] `scripts/soak-daemon.sh` checks memory and descriptor growth under
      typing, reloads, and control requests (3 minutes in CI). Run the 24 h /
      72 h release soak on real hardware and record the results; it does not
      yet cover compositor restart, portal revocation, or suspend/resume.

### 8. Testing gaps that need hardware or deeper tooling

- [x] An injectable keyboard-descriptor seam exercises real poll readiness,
      event delivery, disconnect removal, and matcher reset using fake streams.
- [ ] Kernel `/dev/input` discovery, physical multi-keyboard hotplug, and
      suspend/resume still need a hardware lab; fake descriptors cannot certify
      kernel or device lifecycle behavior.
- [x] The evdev poll helper's eventfd readiness and drain behavior is covered
      by a synthetic test without opening a kernel device.
- [x] The input-method poll helper's eventfd readiness and drain behavior is
      covered with synthetic descriptors, without connecting to a compositor.
- [x] An injectable input-method event-transport seam exercises readiness,
      activation, safe content-type `done`, deactivation, dispatch failure, and
      flush failure, and disconnect through `InputMethodSource`; live
      Wayland/compositor behavior remains a separate certification requirement.
- [ ] Loom-style modelling of the waker, output-completion, and completion
      notifier paths would need `cfg(loom)` shims for std sync types.
- [ ] Form snippets and the clipboard variable need validation with real
      clients on each compositor (focus return after the form closes).

## Security and product gates

### Action broker

The optional action-broker design remains a security gate for networked or
credentialed command workflows. See
docs/ACTION_BROKER_ARCHITECTURE.md.

- [x] Bounded, authenticated, action-name-based IPC and daemon routing are
      implemented through the standalone broker service. Named actions fail
      closed when the broker is unavailable; direct commands remain a separate
      explicitly configured path.
- [x] Broker-side executable ownership, permission, working-directory, timeout,
      environment, output, and process-group checks are implemented.
- [x] The packaged broker service has an independent systemd sandbox with
      explicit no-network (`AF_UNIX` only), read-only-home, and restricted
      filesystem policy; its contract is tested in CI.
- [ ] Add per-action OS sandbox profiles or a container boundary for stronger
      isolation between configured actions; same-UID software and custom broker
      launches remain outside the packaged unit's trust boundary.
- [x] Optional execution audit logging has bounded rotation, privacy-preserving
      records, queue and persistence-failure reporting, and tests. Operational
      deployment and lifecycle certification remain release work.
- [x] Daemon policy routing fails closed for named actions when the broker is
      unavailable; the managed service lifecycle still needs certification.

### Distribution and release evidence

- [ ] Publish installable release assets. Every tag-triggered Release run
      since v1.2.0 has failed (v1.3.3 failed in "Package aarch64 release"),
      so GitHub releases have no `.deb`, `.rpm`, or tarball assets even
      though the packaging and workflows exist.

- [ ] Keep GitHub, Debian/Launchpad, RPM, Arch, and AppStream metadata
      synchronized for each release.
- [ ] Publish only artifacts that pass the vendored offline-build checks.
- [ ] Track package availability and certification status separately.
- [ ] Add release/upgrade smoke tests for supported distributions.

## Product roadmap

Shipped in the current cycle: trigger aliases, `wayexpand explain`,
capability-aware template variables (includes, allowlisted env, opt-in
clipboard), interactive form snippets, local usage statistics and a trigger
risk analyzer, Git library sync, and signed organization packs.

Still open:

- [ ] **Next after compositor certification:** cross-desktop application
      identity so app-filtered snippets,
      Quick Picker direct insertion, and form return-to-origin work beyond
      KWin: `wlr-foreign-toplevel-management` where available, compositor IPC
      adapters (Sway, Hyprland) as fallbacks, and an optional GNOME Shell
      extension exposing a small authenticated window-identity API. Missing
      identity must keep failing closed.
- [ ] Publish a canonical `route_id` in daemon status (additive schema change)
      so consumers stop reconstructing routes from `source`/`backend`.
- [ ] `{{clipboard}}` runs `wl-paste` on demand (bounded at 150 ms) while
      rendering. Reading on demand is deliberate: a background clipboard
      monitor would keep every copied item, including passwords, in daemon
      memory. Revisit only with a design that preserves that property (for
      example a short-lived read started when a clipboard trigger prefix is
      typed).
- [ ] Build one `ValidatedConfig` (effective triggers, compiled app filters,
      template library and parsed templates, parsed hotkeys, normalized IDs,
      policy-resolved settings) during validation, and have the engine, reload
      and GUI consume it instead of recompiling, so validation and runtime
      cannot disagree. Effective triggers
      are already reused; app filters, hotkeys and IDs are still recompiled.
- [ ] Profiles/workspaces (personal, work, support) switchable at runtime.
- [ ] Broader imports (TextExpander, AutoKey, aText, CSV).
- [ ] Form snippets through IBus (the route drops surrounding text when focus
      moves to the form) and a GUI editor for form fields.
- [ ] Signed team-pack registry: `wayexpand pack update`, `pack diff`,
      `pack rollback`, and `pack verify` against organization trust roots,
      plus a GUI trust review for signed packs.
- [ ] Parameterized broker actions: a per-action parameter schema (for
      example `type = "enum"` with fixed values) that validates every input
      before it is placed in a fixed argument slot, so forms can drive
      allowlisted, shell-free commands. No free-form interpolation.
- [ ] Runtime keyboard-layout updates for evdev capture (today a layout change
      needs a daemon restart).
- [ ] Real external IME/preedit cooperation (see section 6).

## Maintainability

Not release blockers; they keep 2.0 tractable.

- [ ] Continue splitting oversized modules: `core/src/engine/mod.rs`,
      `core/src/config.rs` (model, limits, validation, templates, storage,
      migration), `backend-libei/src/lib.rs` (connection, portal, token,
      keymap, text, keyboard fallback, injector), `gui/src/app/editor.rs`,
      `backend-input-method/src/lib.rs`, and `backend-ibus/src/lib.rs`.
- [ ] Move GUI strings out of the Rust `lang.rs` match tables into a
      data-driven format (for example Fluent/FTL) before adding languages.

## Maintenance principles

- Keep this file limited to incomplete work and measurable exit criteria.
- Put completed work and superseded plans in the changelog or archive.
- Update docs/SUPPORT_MATRIX.md only when reproducible evidence changes.
- Never describe an experimental backend as certified without exact test
  evidence.

## References

- docs/SUPPORT_MATRIX.md — current support status
- docs/CERTIFICATION_MATRIX.md — backend selection and compositor evidence
- docs/CERTIFICATION.md — certification procedure
- docs/INTEGRATION_TESTING.md — integration tests
- docs/COMPATIBILITY.md — stable contracts
- docs/PACKAGING.md — distribution guidance
