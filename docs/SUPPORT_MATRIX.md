# Support Matrix

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Support Matrix**

---

This matrix separates implemented code from verified desktop behavior. A
Wayland session alone does not imply that a backend is usable.

**Core engine and config are stable; desktop backend support is compositor-dependent.** See [CERTIFICATION_MATRIX.md](CERTIFICATION_MATRIX.md) for the authoritative automatic-selection and certification status. No compositor is certified by automated end-to-end tests yet. Run `wayexpand doctor` on your own session before relying on capture.

**Product scope (current):** WayExpand is intended for direct keyboard input
and text that an input method has already committed. Active external
IME/preedit composition is explicitly unsupported. Local XKB dead-key and
Compose sequences are treated as composition boundaries and are only matched
after their committed Unicode text arrives. This is a scope boundary, not a claim
that any desktop is currently production-certified; certification applies only
to the tested backend, compositor, client, and layout combinations.

| Area | Current status | Evidence required for promotion |
| --- | --- | --- |
| Core matching and config validation | Supported | Workspace unit tests and Clippy |
| CLI, JSON diagnostics, and config tooling | Supported | CLI and daemon smoke tests |
| systemd user services | Supported | `systemd-analyze verify` and installer test |
| wlroots virtual-keyboard output | Experimental | Target-compositor insertion test |
| libei/EIS output | Experimental | Portal consent, revocation, and reconnect tests; on-device tests for the `ei_keyboard`-only keysym-synthesis fallback (no `ei_text`), including non-US layouts |
| input-method-v2 capture | Experimental | Activation, focus, and Unicode tests |
| evdev capture (`--source=evdev --allow-evdev-sensitive-fields`) | Experimental, best-effort | Compositor-agnostic compatibility fallback (e.g. KWin); requires explicit raw-input consent plus an explicit acknowledgement that it has no sensitive-field signal, and cannot make rapid replacement atomic under non-exclusive capture. See [`BACKENDS.md`](BACKENDS.md), [ADR 0001](adr/0001-evdev-best-effort-semantics.md), and [`SECURITY.md`](../SECURITY.md) |
| Global hotkeys | Experimental | Backend capture and action tests |
| Focused-window tracking (`app_filter`) | Supported by the KDE/KWin path; awaiting independent certification | KWin tracker exists; wlroots and GNOME paths explicitly report application filters unavailable rather than guessing |
| Key pass-through | Experimental | libei-assisted lifecycle-aware press/release pass-through exists for input-method-v2; modifier chords, repetition, reconnect, and compositor/client behavior still require certification |
| Runtime keyboard-layout switching | Route-dependent | input-method-v2 and IBus expose layout-aware input paths; evdev uses a startup-only local XKB snapshot and must be restarted after layout changes |
| External preedit/IME composition | **Not supported** | ⚠️ Affects CJK and active Fcitx/IBus/Rime composition (see below) |

## Desktop coverage

No compositor is currently certified by automated end-to-end tests. Before
deploying desktop capture, test and record the exact compositor and version:

The table below is generated from the certification target contract. Its
paths are test targets, not a guarantee that the path is available or works on
your session. E2E certification changes only after reviewed real-session
evidence exists.

<!-- generated:desktop-certification-matrix:start -->
| Target | Desktop/session | Declared test paths | Window tracking | App filters | Sensitive fields observed | E2E certification |
| --- | --- | --- | --- | --- | --- | --- |
| `kde` | KDE Plasma / KWin | ibus, evdev+libei, input-method-v2 | KWin application tracker | Available in declared path | Not observed | **Not certified** |
| `gnome` | GNOME Shell / Mutter | ibus, evdev+libei, input-method-v2 | none | Unavailable | Not observed | **Not certified** |
| `sway` | Sway / wlroots | evdev+wlroots | none | Unavailable | Not observed | **Not certified** |
| `hyprland` | Hyprland / wlroots | evdev+wlroots | none | Unavailable | Not observed | **Not certified** |
<!-- generated:desktop-certification-matrix:end -->

<!-- generated:route-contract:start -->
| Route | Capture | Injection | Sensitive fields (implementation) | Protocol signal | Compositor observation | Certification | Atomic replace | App identity | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `ibus` (IBus) | ibus | ibus | implemented | IBus content purpose | session-dependent | not certified | no | none | experimental |
| `kde-evdev-libei` (Maximum compatibility) | evdev | libei | not implemented | none | unavailable | not certified | no | kwin | experimental |
| `sway-evdev-wlroots` (Evdev + wlroots) | evdev | wlroots-virtual-keyboard | not implemented | none | unavailable | not certified | no | none | experimental |
| `input-method-v2` (Input Method v2) | input-method-v2 | input-method-v2 | implemented | input-method-v2 content purpose | compositor-dependent | not certified | yes | none | experimental |
<!-- generated:route-contract:end -->

Keyboard-layout evidence is mandatory for certification: `us`, `de`, `fr`, an
AltGr-heavy layout, and a multi-layout switching setup. A US-only run is not
evidence for layout-independent text injection. Certification checks
dead-key/Compose input and expansion only after composition is committed; the
local keyboard-composition guard prevents a trigger from being matched while
those sequences are active. External active preedit is an explicit out-of-scope capability and does not become
supported through a passing certification. The machine-readable report lists
this boundary under `out_of_scope_capabilities`.

The integration procedure is in
[`docs/INTEGRATION_TESTING.md`](INTEGRATION_TESTING.md). A passing unit test,
backend probe, or running systemd process is not certification evidence.

## Promotion policy

A backend is promoted to Supported only after reproducible tests cover normal
typing, Unicode, deletion, selections, password fields, application
shortcuts, focus changes, compositor restart, and configuration reload. Known
limitations must be visible in `wayexpand doctor`, the GUI, and the release
notes.

## Known Limitations: Active IME and Preedit Composition

**WayExpand does not support external active Input Method Editor (IME)
composition or preedit sequences.**

This affects users who rely on:

- **CJK input** (Chinese, Japanese, Korean) using active IME composition
- **External IME preedit** for CJK and other active composition engines
- **Multi-key sequences** (e.g., Compose key combinations)
- **Active IME/preedit composition workflows** (e.g., Fcitx, IBus engines, Rime, and similar systems actively composing text)

### Why Unsupported

The supported workflow is direct keyboard input or text expansion after an
input method has committed its text. WayExpand safely recognizes local XKB
dead-key/Compose boundaries, but does not observe or control another engine's
active IME session. Typing an expansion trigger during external IME
composition may:

- Interrupt ongoing composition
- Insert the trigger text before the IME has finished
- Produce garbled output

### Workarounds

1. **Complete composition first:** Finish your IME input (Enter/Space), then type expansion triggers
2. **Use global shortcuts instead:** Configure a hotkey that directly expands without typing
3. **Separate tools:** Use your IME system's own abbreviation/phrase expansion (many have this built-in)

### About WayExpand's IBus Support

WayExpand ships its own **IBus engine backend** (`wayexpand-ibus`). This is an
alternative WayExpand input path, not support for observing or coordinating
another engine's active preedit state. Its behavior still requires testing with
the target IBus version and client toolkit. The committed-text scope applies
when text has already been committed, subject to the selected backend/client
path being tested:
- After composition is committed (Enter/Space)
- Outside of active composition sessions
- Via the WayExpand IBus engine when selected and configured

This is not a blanket statement that IBus is unsupported: WayExpand has an
IBus integration. It does not promise interoperability with active composition
from Fcitx, IBus engines, Rime, or other IMEs unless that exact path is tested.

### For Package Maintainers / System Administrators

When deploying WayExpand in regions or environments with heavy IME usage:

- **Document the limitation clearly** in your deployment guides (specifically: active composition incompatibility)
- **Test with your local IME** before recommending to users
- **Describe the IBus engine accurately:** It is an available integration path, not evidence of active-preedit support or universal client compatibility
- **Suggest complementary tools:** Many input methods (Fcitx, IBus) have built-in phrase expansion that can complement WayExpand

### Planned Architecture Work

Composition support is a deliberate future engineering direction, but it is
not implemented and has no release guarantee. The staged architecture,
security invariants, protocol-selection caveats, and promotion gates are in the
[IME and composition roadmap](IME_COMPOSITION_ROADMAP.md). In particular,
text-input-v3 is not by itself a universal global input-observation API, and
passing protocol tests does not establish compositor/client compatibility.

### Check Your Environment

To see if IME composition affects you:

```bash
# If you're using an IME, you'll see it active here
systemctl --user status fcitx.service  # or ibus, etc.

# WayExpand will still work, but composition + expansion may not mix
wayexpand doctor
```

**Summary:** The current intended scope is direct keyboard input and committed
text. Active-preedit expansion is unsupported; users who need expansion during
composition should use their IME's own phrase-expansion feature or a
composition-aware tool. Desktop behavior remains uncertified until real-client
evidence is published.
