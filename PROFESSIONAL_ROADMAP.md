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

### 5. IME and preedit strategy

Preedit/IME composition is currently unsupported. Decide and document the
supported end state before broad adoption:

- [ ] Evaluate native text-input/IME integration for composition-aware
      expansion.
- [ ] Define an explicit Fcitx/Rime/IBus strategy.
- [ ] Certify Chinese, Japanese, Korean, dead-key, and Compose workflows or
      document their supported fallback behavior.
- [ ] Keep the limitation prominent in docs/SUPPORT_MATRIX.md until evidence
      exists.

### 6. Crash, restart, and lifecycle soak testing

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

### 7. Testing gaps that need hardware or deeper tooling

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
      disconnect through `InputMethodSource`; live Wayland/compositor behavior
      remains a separate certification requirement.
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

- [ ] Better cross-desktop application identity (GNOME and wlroots window
      tracking) so app-filtered snippets work beyond KWin.
- [ ] Profiles/workspaces (personal, work, support) switchable at runtime.
- [ ] Broader imports (TextExpander, AutoKey, aText, CSV).
- [ ] Form snippets through IBus (the route drops surrounding text when focus
      moves to the form) and a GUI editor for form fields.
- [ ] Pack update channel and a GUI trust review for signed packs.
- [ ] Real external IME/preedit cooperation (see section 5).

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
