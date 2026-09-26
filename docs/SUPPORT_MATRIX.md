# Support Matrix

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Support Matrix**

---

This matrix separates implemented code from verified desktop behavior. A
Wayland session alone does not imply that a backend is usable.

**Core engine and config are stable; desktop backend support is compositor-dependent.** See [COMPOSITOR_MATRIX.md](COMPOSITOR_MATRIX.md) for the authoritative automatic-selection and certification status. No compositor is certified by automated end-to-end tests yet. Run `wayexpand doctor` on your own session before relying on capture.

| Area | Current status | Evidence required for promotion |
| --- | --- | --- |
| Core matching and config validation | Supported | Workspace unit tests and Clippy |
| CLI, JSON diagnostics, and config tooling | Supported | CLI and daemon smoke tests |
| systemd user services | Supported | `systemd-analyze verify` and installer test |
| wlroots virtual-keyboard output | Experimental | Target-compositor insertion test |
| libei/EIS output | Experimental | Portal consent, revocation, and reconnect tests; on-device tests for the `ei_keyboard`-only keysym-synthesis fallback (no `ei_text`), including non-US layouts |
| input-method-v2 capture | Experimental | Activation, focus, and Unicode tests |
| evdev capture (`--source=evdev`) | Experimental, best-effort | Compositor-agnostic compatibility fallback (e.g. KWin); requires explicit raw-input consent, has no sensitive-field signal, and cannot make rapid replacement atomic under non-exclusive capture. See [`BACKENDS.md`](BACKENDS.md), [ADR 0001](adr/0001-evdev-best-effort-semantics.md), and [`SECURITY.md`](../SECURITY.md) |
| Global hotkeys | Experimental | Backend capture and action tests |
| Focused-window tracking (`app_filter`) | Supported by the KDE/KWin path; awaiting independent certification | KWin tracker exists; wlroots and GNOME paths explicitly report application filters unavailable rather than guessing |
| Key pass-through | Experimental | libei-assisted lifecycle-aware press/release pass-through exists for input-method-v2; modifier chords, repetition, reconnect, and compositor/client behavior still require certification |
| Preedit/IME composition | **Not supported** | ⚠️ Affects CJK, dead-keys, composition (see below) |

## Desktop coverage

No compositor is currently certified by automated end-to-end tests. Before
deploying desktop capture, test and record the exact compositor and version:

- Sway / wlroots
- Hyprland / wlroots
- KDE Plasma Wayland
- GNOME Shell Wayland

Keyboard-layout evidence is mandatory for certification: `us`, `de`, `fr`, an
AltGr-heavy layout, and a multi-layout switching setup. A US-only run is not
evidence for layout-independent text injection. CJK/IME and preedit remain
unsupported categories until they receive a separate implementation and
certification plan.

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

**WayExpand does not support active Input Method Editor (IME) composition or preedit sequences.**

This affects users who rely on:

- **CJK input** (Chinese, Japanese, Korean) using active IME composition
- **Dead-key composition** (accented characters: é, ñ, ü, etc.) during composition
- **Multi-key sequences** (e.g., Compose key combinations)
- **Active IME/preedit composition workflows** (e.g., Fcitx, IBus engines, Rime, and similar systems actively composing text)

### Why Unsupported

Text expansion happens at the character/string level, after the input method has committed text. WayExpand's replacement injection cannot interact with active composition sessions — typing an expansion trigger during IME composition may:

- Interrupt ongoing composition
- Insert the trigger text before the IME has finished
- Produce garbled output

### Workarounds

1. **Complete composition first:** Finish your IME input (Enter/Space), then type expansion triggers
2. **Use global shortcuts instead:** Configure a hotkey that directly expands without typing
3. **Separate tools:** Use your IME system's own abbreviation/phrase expansion (many have this built-in)

### About WayExpand's IBus Support

WayExpand ships its own **IBus engine backend** (`wayexpand-ibus`). This backend works alongside IBus for text expansion. The limitation documented above applies specifically to **active composition** — when the IME is actively composing (in preedit state). Text expansion works fine:
- After composition is committed (Enter/Space)
- Outside of active composition sessions
- Via the native IBus engine that WayExpand provides

This is different from saying "IBus is unsupported." Rather, WayExpand and IBus composition don't safely mix during active composition, but they coexist fine otherwise.

### For Package Maintainers / System Administrators

When deploying WayExpand in regions or environments with heavy IME usage:

- **Document the limitation clearly** in your deployment guides (specifically: active composition incompatibility)
- **Test with your local IME** before recommending to users
- **Highlight the IBus engine:** Users in IBus environments can use WayExpand's native IBus backend for seamless integration
- **Suggest complementary tools:** Many input methods (Fcitx, IBus) have built-in phrase expansion that can complement WayExpand

### Future Possibility

Supporting preedit composition would require:
- Native IME protocol integration (Wayland text-input-v3 extensions)
- Compositor-specific testing (GNOME IM, KDE IM, Fcitx, etc.)
- Careful interaction with active composition state

This is tracked as a future enhancement but is not on the current roadmap. Contributions welcome.

### Check Your Environment

To see if IME composition affects you:

```bash
# If you're using an IME, you'll see it active here
systemctl --user status fcitx.service  # or ibus, etc.

# WayExpand will still work, but composition + expansion may not mix
wayexpand doctor
```

**Summary:** WayExpand is great for ASCII-heavy English/European languages and global hotkeys. For CJK, dead-keys, or composition-heavy workflows, consider your IME system's built-in expansion features as a complement or alternative.
