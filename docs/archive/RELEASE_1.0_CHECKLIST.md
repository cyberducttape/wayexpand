# 1.0 release checklist

This is the milestone-level bar for calling a release "1.0" -- distinct from
[`docs/wiki/Contributing.md`](wiki/Contributing.md)'s per-PR checklist and
[`docs/RELEASING.md`](RELEASING.md)'s per-tag mechanics, both of which still
apply to every 1.0 release candidate on top of this.

[`docs/RELEASING.md`](RELEASING.md) already states the governing rule:

> Do not call a release stable while the support matrix still marks key
> pass-through or compositor coverage as unsupported.

Every item below exists to make that rule checkable rather than a judgment
call. An item is done when its **Pass/fail** condition is met, not when the
underlying work merely happened. Where an item needs a human sitting in a
real desktop session, this repository's automation cannot satisfy it --
**Owner** says who has to.

## 1. Compositor certification

The single largest gap. [`docs/SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md)
currently marks every input/output backend "Experimental," and CI only runs
`cargo test`/`clippy` on `ubuntu-latest` with no real compositor. Promotion
to "Supported" happens per
[`docs/INTEGRATION_TESTING.md`](INTEGRATION_TESTING.md)'s promotion policy,
which requires normal typing, Unicode, deletion, selections, password
fields, application shortcuts, focus changes, compositor restart, and
configuration reload -- a passing unit test or a running systemd process is
explicitly not certification evidence.

- [ ] **Sway (wlroots)** -- `--source=input-method` or `--source=evdev`,
      paired with `--backend=wlroots` and `--backend=libei`, each walked
      through `docs/INTEGRATION_TESTING.md` in full.
      **Owner:** human, real Sway session. **Pass/fail:** every checklist
      item passes; failures are either fixed or added to
      `SUPPORT_MATRIX.md` as a named limitation.
- [ ] **Hyprland (wlroots)** -- same procedure as Sway.
      **Owner:** human, real Hyprland session.
- [ ] **KDE Plasma / KWin** -- `--source=evdev` (KWin 6.6 advertises neither
      input-method-v2 nor virtual-keyboard), both output backends, plus
      `app_filter`/`wayexpand-backend-kwin-window` specifically re-verified
      on whatever KWin version is current at release time (built today
      against 6.6.6 only -- KWin 5's scripting engine differs and is
      untested).
      **Owner:** human, real KWin session.
- [ ] **GNOME Shell (Mutter)** -- same procedure; GNOME is also the
      remaining gap for `app_filter` (no tracker implemented -- see §5).
      **Owner:** human, real GNOME session.
- [ ] For each compositor above, `SUPPORT_MATRIX.md`'s row is updated from
      "Experimental" to "Supported" (or left Experimental with the specific
      observed gap named) based on that session's actual results, not
      aspirationally in advance.
      **Pass/fail:** matrix text matches what was actually observed, with
      the compositor/version recorded per `INTEGRATION_TESTING.md`'s
      "Record the compositor, desktop session, keyboard layout" instruction.
- [ ] Global hotkeys (`InputEvent::Key` / hotkey actions) exercised on each
      compositor above, not just the core engine's dispatch-layer unit
      tests.
- [ ] Key pass-through and preedit/IME composition: explicit decision
      recorded (ship 1.0 without them, documented as known gaps in the
      release notes and `SUPPORT_MATRIX.md`; or block 1.0 until resolved).
      **Pass/fail:** the decision is written down somewhere a user reads
      before installing, not just implied by matrix silence.

## 2. Stability guarantees

A 1.0 version number implies a promise about what won't break in 1.x. None
of the following is written down yet.

- [ ] Write `docs/COMPATIBILITY.md` (or a section in `RELEASING.md`) stating
      what's covered by semver going forward: at minimum, the TOML config
      schema (existing fields, `#[serde(default)]` behavior for new ones),
      the `list --json` / `preview --json` / `doctor --json` /
      `status --json` output shapes, and CLI exit codes
      (`EXIT_USAGE`/`EXIT_CONFIG`/`EXIT_DAEMON` and friends).
      **Pass/fail:** a third party building a settings UI or CI check
      against these surfaces can tell, from the doc alone, what they can
      rely on.
- [x] Confirm every new field added this session
      (`ExpansionConfig::category`, `ExpansionConfig::app_filter`) round-trips
      through an *old* config file with the field absent -- covered by
      `#[serde(default)]` and now an explicit regression test
      (`config::tests::pre_category_config_without_new_fields_still_parses`)
      that loads a pre-category config literal and asserts it still parses.
- [ ] Decide and document the config migration story for a hypothetical
      future breaking field change (major-version bump required, or an
      in-place migration path). Doesn't need to be built yet, needs to be
      decided.

## 3. Security review

The project markets itself as privacy-first and captures every keystroke
when active; `SECURITY.md` documents the threat model and the no-shell
command-execution guarantee, but has not had outside verification.

- [ ] External security review (or at minimum a structured self-audit
      against `SECURITY.md`'s stated model) covering: the no-shell
      `Command::new(program).args(args)` execution path (§SECURITY.md,
      confirmed by this session's engine.rs read: `run_command`/
      `execute_hotkey`), config file ownership/permission checks in
      `config.rs`, the Unix control socket's permission model, and the new
      `backend-kwin-window` D-Bus service (a per-process uniquely-named
      bus name and object; confirm nothing else on the session bus can
      spoof or intercept its callback).
      **Owner:** ideally someone other than whoever wrote the code being
      reviewed.
- [ ] Command-backed snippets and hotkeys: `SECURITY.md` already states this
      is user-supplied executable content protected only by file ownership
      checks, not a runtime confirmation gate. Explicit 1.0 decision: ship
      as-is (documented, matches AutoKey/Espanso's equivalent trust model)
      or add a first-run confirmation/allowlist. Either is defensible;
      leaving it undecided is not.
- [ ] Dependency audit: `cargo audit` (or equivalent, the supply-chain CI
      job already runs something similar) result reviewed by a human, not
      just green-checked by CI, given the number of new dependencies this
      session added (`zbus` and its transitive tree, `x11-clipboard`).

## 4. Packaging currency

- [x] Fedora Copr: `README.md` no longer shows a runnable `dnf copr enable`
      command for a repository that doesn't exist; it now says plainly that
      no Copr repo exists yet and points at PACKAGING_CHECKLIST.md.
- [ ] PPA (`ppa:cyberducttape/ppa`) and AUR rebuilt and verified against the
      current workspace, which gained two crates this session
      (`crates/backend-clipboard`, `crates/backend-kwin-window`) not
      present when those packages were last verified. Confirm
      `debian/control`/`debian/rules` and the AUR `PKGBUILD` pick up both
      via the workspace rather than needing per-crate additions (this
      session's `debian/rules` changes already add the new icon assets;
      verify a clean `dpkg-buildpackage` actually produces a working
      `.deb`, not just that the rules file parses).
      **Owner:** human with access to the PPA/AUR accounts, or a fresh
      isolated build environment.
- [ ] `scripts/test-release.sh` and `scripts/test-install-release.sh` run
      against an actual tagged build, not just the working tree.

## 5. This session's own follow-through items

Concrete, known gaps from the `app_filter`/window-tracker and GUI work done
today -- listed here so they don't silently become permanent "temporary"
limitations.

- [ ] `app_filter` window tracking: KDE Plasma only. wlroots compositors
      have a real, standard protocol available
      (`wlr-foreign-toplevel-management-unstable-v1`, already vendored via
      `wayland-protocols-wlr` in `Cargo.lock`) and are not yet implemented.
      GNOME has no equivalent without a shell extension. Decide: block 1.0
      on at least the wlroots tracker (protocol is standard, lower risk
      than the KWin D-Bus bridge), or ship 1.0 with `app_filter` explicitly
      scoped as "KDE Plasma only" in release notes.
- [ ] `backend-kwin-window`'s `run()` retry-after-`loadScript` race was
      observed and worked around empirically (15 attempts, 150ms backoff)
      against one live KWin 6.6.6 session in this conversation. Re-verify
      against whatever KWin version ships with the target distros at
      release time; the race margin could differ.
- [ ] GUI's "Use current app" button blocks the UI thread for the full
      `KwinWindowTracker::new()` + first-callback round trip (roughly
      1-2s observed). Not release-blocking, but a real papercut -- consider
      a background thread + spinner if it's still this slow at 1.0.
- [ ] `crates/backend-kwin-window`'s D-Bus service name
      (`org.wayexpand.WindowTracker.pid<pid>`) and script plugin name are
      process-scoped for collision-avoidance, but nothing currently cleans
      up a `/tmp/wayexpand-window-tracker-<pid>.js` file or an unloaded
      script left behind by a daemon that was `kill -9`'d rather than
      shut down cleanly. Low severity (next boot's differing PID avoids
      collision), but worth a documented cleanup story or an accepted
      known limitation.

## 6. Documentation pass

- [ ] `README.md`'s "Current milestone" bullet list (last touched this
      session) reviewed end-to-end for anything still describing
      pre-1.0/experimental behavior as if final.
      Cross-check against whatever `SUPPORT_MATRIX.md` says after §1 above
      -- the two must agree.
- [ ] `docs/wiki/` walked page-by-page against the actual running 1.0
      candidate build, not just the pages touched this session
      (`GUI.md`, screenshots).
- [ ] `CHANGELOG.md`'s `Unreleased` section fully moved into a `1.0.0`
      section per `RELEASING.md` step 4, written for a user deciding
      whether to upgrade, not as a commit-log dump.

## 7. Mechanical release gate (already defined elsewhere, listed for completeness)

- [ ] Everything in [`docs/wiki/Contributing.md`](wiki/Contributing.md)'s
      "Release checklist" passes with `--locked --release`.
- [ ] [`docs/RELEASING.md`](RELEASING.md) followed exactly, including the
      isolated `scripts/test-release.sh` smoke test and the tag/version
      match enforced by the release workflow.
