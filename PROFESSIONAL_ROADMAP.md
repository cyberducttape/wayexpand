# WayExpand Professional Roadmap

> This is the forward-looking roadmap. Completed work and historical release
> planning are archived in [docs/archive/2026-09/PROFESSIONAL_ROADMAP.md](docs/archive/2026-09/PROFESSIONAL_ROADMAP.md);
> current behavior is defined by the support and compatibility documents.

The core engine, configuration format, and CLI/JSON contracts are stable for
the 1.2.x line. Desktop backend support remains experimental until the
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

## Security and product gates

### Action broker

The optional action-broker design remains a security gate for networked or
credentialed command workflows. See
docs/ACTION_BROKER_DESIGN.md.

- [ ] Bounded, authenticated, action-name-based IPC.
- [ ] Independent sandbox and ownership checks.
- [ ] Per-action filesystem, network, environment, timeout, output, and audit
      policy.
- [ ] Fail-closed denial and unavailability behavior.

### Distribution and release evidence

- [ ] Keep GitHub, Debian/Launchpad, RPM, Arch, and AppStream metadata
      synchronized for each release.
- [ ] Publish only artifacts that pass the vendored offline-build checks.
- [ ] Track package availability and certification status separately.
- [ ] Add release/upgrade smoke tests for supported distributions.

## Maintenance principles

- Keep this file limited to incomplete work and measurable exit criteria.
- Put completed work and superseded plans in the changelog or archive.
- Update docs/SUPPORT_MATRIX.md only when reproducible evidence changes.
- Never describe an experimental backend as certified without exact test
  evidence.

## References

- docs/SUPPORT_MATRIX.md — current support status
- docs/COMPOSITOR_MATRIX.md — backend selection and compositor evidence
- docs/CERTIFICATION.md — certification procedure
- docs/INTEGRATION_TESTING.md — integration tests
- docs/COMPATIBILITY.md — stable contracts
- docs/PACKAGING.md — distribution guidance
