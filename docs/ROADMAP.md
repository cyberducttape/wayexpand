# WayExpand Roadmap

> This is the forward-looking roadmap. Completed work and historical release
> planning are archived in [docs/archive/2026-09/PROFESSIONAL_ROADMAP.md](archive/2026-09/PROFESSIONAL_ROADMAP.md);
> current behavior is defined by the support and compatibility documents.

As of v1.3.3, the core engine, configuration format, and CLI/JSON contracts
are stable. Desktop backend support remains experimental until the
certification evidence described in [CERTIFICATION.md](CERTIFICATION.md)
and [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) is published.

## Production blockers

These are the work items that most directly determine whether WayExpand is
safe to recommend for everyday desktop use.

### 1. KDE/KWin certification

- [ ] Run the complete end-to-end matrix on a supported KDE Plasma/KWin version.
- [ ] Cover Firefox, Chromium, Qt, GTK, Electron, terminals, Unicode, rapid
      typing, shortcuts, focus changes, layout changes, restart, suspend/resume,
      portal revocation, and device reconnect.
- [ ] Publish exact versions, backend choices, test dates, and known limitations.
- [x] Headless real-client driver for the `evdev+libei` path
      (`scripts/kwin-certification-driver.sh`): GTK, Qt, Konsole, gedit and
      Kate in private KWin sessions across all five layout profiles. Output,
      focus and daemon-lifecycle cells only; see
      [CERTIFICATION.md](CERTIFICATION.md#headless-kwin-real-client-driver).
- [ ] Capture-side cells need a live-session driver: physical keyboard
      capture, key pass-through, pending-output concurrency through the evdev
      pipeline, hardware reconnect, suspend/resume, portal revocation.
- [ ] Browser and Electron cells: the snap Firefox/Chromium cannot join a
      private compositor; drive non-snap builds or a live session.

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
      and reconnects when its device is paused or removed. Headless KWin
      6.6.6 never reports the active layout of a `us,de` keymap, even after
      a switch, so nothing is typed with several layouts configured (see
      SUPPORT_MATRIX.md). Confirm in a live session, then raise it with KWin
      or derive the group from another trusted source.
- [x] Multi-layout and unrepresentable-text refusals are checked during
      transaction preflight, before a non-atomic route erases the trigger;
      these failures are classified as "not applied". The fallback still
      refuses to type on KWin when EIS does not report the active group.

### 5. Certification-driven route planner

`plan_routes()` ranks every catalog route from the capability snapshot,
organization policy, and route certification status: certified routes first,
then input-method-v2 (with libei pass-through), then IBus; raw evdev routes
always require explicit consent.

- [ ] Feed local certification evidence (from `wayexpand doctor --certify`
      records) into the planner, not only the catalog status.
- [ ] Add Fcitx5 as a candidate once an integration exists.
- [x] Show the full ranked plan (standing and reason per route) in setup, the
      GUI diagnostics, and `doctor`; all three use the shared planner and keep
      catalog certification separate from runtime availability.

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
- [ ] Loom models of small extracted state machines (needs `cfg(loom)` shims
      for std sync types), in priority order: output completion
      notification, daemon shutdown during a pending injection, async command
      completion during reload, waker delivery, broker shutdown, and GUI save
      completion versus an external file edit.
- [ ] Form snippets and the clipboard variable need validation with real
      clients on each compositor (focus return after the form closes).
      Headless KWin and Sway now run `clipboard-variable` and
      `clipboard-empty-fails-closed`; KWin passes since the daemon reads the
      clipboard over `ext-data-control-v1` (wl-clipboard 2.2.1 took focus on
      every read). `form-returns-to-origin` and
      `form-focus-moved-fails-closed` run on the KWin input-method lane, which
      stays unverified until a session grants the daemon input-method-v2.
- [ ] Run the certification matrix in remote-desktop sessions (RDP/VNC via
      the compositor's remote-desktop portal) and nested compositor sessions,
      and record whether each route is supported there.

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
- [ ] Extend per-action isolation beyond the initial Landlock filesystem
      profiles. Packaged broker actions now run with Landlock read/write
      grants; profiles fail closed when enforcement is unavailable. The
      service keeps network disabled globally. Per-action TCP policy, device
      allowlists (including global device isolation), and action-specific
      seccomp syscall sets remain open; same-UID software and custom broker
      launches remain outside the packaged unit's trust boundary.
- [x] Optional execution audit logging has bounded rotation, privacy-preserving
      records, queue and persistence-failure reporting, and tests. Operational
      deployment and lifecycle certification remain release work.
- [x] Daemon policy routing fails closed for named actions when the broker is
      unavailable; the managed service lifecycle still needs certification.

### Distribution and release evidence

- [ ] Publish installable release assets. The Release workflow requires the
      real compositor-certification workflow to pass before either release
      architecture can build or publish. Currently no route in
      `tests/certification/compositor-matrix.json` is production eligible, so
      a tag cannot reach packaging until a route passes the production
      certification contract. The aarch64 job builds and checks every release
      binary, and the x86_64 publish job still needs an end-to-end run after
      certification is unblocked. Launchpad source-package synchronization is
      a separate workflow and does not publish GitHub release assets.

- [ ] Keep GitHub, Debian/Launchpad, RPM, Arch, and AppStream metadata
      synchronized for each release.
- [ ] Publish only artifacts that pass the vendored offline-build checks.
- [ ] Track package availability and certification status separately.
- [ ] Add release/upgrade smoke tests for supported distributions. CI's
      `package-smoke` job installs the built `.deb` on Ubuntu 24.04 and
      Debian 13 and the `.rpm` on Fedora, checks shared-library resolution,
      versions, user units and example validation, reinstalls, and removes
      the package. Upgrade from a previously published package remains open:
      no release has published packages yet.

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
- [x] Publish a canonical `route_id` in daemon status (schema 11); the GUI
      resolves route contracts from that ID instead of reconstructing routes
      from `source`/`backend`.
- [ ] Complete the shared immutable `ValidatedConfig` pipeline. The snapshot
      now retains the exact source config, resolved organization policy,
      normalized IDs, effective triggers, parsed app filters/form fields,
      template variable metadata, snippet library, and parsed hotkeys. The
      engine retains it and daemon reload builds the engine from the validated
      snapshot; GUI preview uses the same validator through engine creation.
      Runtime host restrictions remain a separate overlay. Template AST
      compilation and reuse during rendering, and end-to-end consistency tests
      across fleet/reload paths remain open.
- [ ] Named full-config profiles support CLI create/list/switch/remove,
      checkpointing on switch and daemon reload. The GUI now lists validated
      profiles and can create or switch them asynchronously, preserving its
      unsaved-edit confirmation and reloading the open library. Still open:
      hotkey switching, profile-specific snippet layers, per-application
      automatic selection, and per-profile rollback history.
- [ ] Broader imports (TextExpander, AutoKey, aText, CSV).
- [x] Form snippets through IBus: completions are bound to the originating
      input context and require surrounding-text verification (non-atomic,
      uncertified).
- [x] A GUI form-field builder inserts validated text, dropdown, and date
      markers at the replacement editor's selection. Advanced rules can still
      be edited directly in the marker; form previews and final saves continue
      through the shared template validator.
- [ ] Richer forms. Shipped: required fields, length limits, choice defaults
      and date fields. Open: clipboard defaults, conditional fields, a date
      picker, and per-field history. Required fields now drive named broker
      actions with enum/text schemas and fixed argv slots; a GUI broker-policy
      schema editor and richer field-to-parameter mapping remain open.
- [ ] Signed team-pack registry: local `pack search` now discovers versioned
      packs in an offline Git checkout and discloses signer status, review
      metadata, capabilities, commands, and hotkeys. `pack diff`, staged
      `pack install`, `pack rollback`, and `pack verify` use existing signer
      policy. The GUI can review local pack contents and diffs, disclose signer
      and capability status, require action consent, and install only the
      reviewed fingerprint. `pack lock`/`lock --check`, `pack outdated`,
      `pack update`, and `pack sync --lock` pin, verify, and update installed
      packs against a catalog by content fingerprint. Still open:
      organization/private registry transport, GUI update review, and
      centralized pack governance.
- [ ] Repository-backed snippet libraries (`wayexpand library add <git url>`)
      building on Git sync: signed-commit verification, branch pinning,
      reviewed/approved updates, diff, rollback, and audit.
- [x] Parameterized broker actions validate enum and bounded text values
      against a per-action schema, replace complete fixed argv slots, and
      accept values from required named forms. Parameterized actions refuse
      shell and language-interpreter executables; no free-form interpolation.
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
