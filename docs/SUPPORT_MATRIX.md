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

Arabic and Chinese characters are accepted in triggers and replacement text
when the selected route reports full-Unicode support. Arabic diacritics are
kept with their grapheme for matching/deletion, and replacement text is passed
to the target application unchanged (which handles its own shaping and text
direction). Chinese entered through an IME is supported only after the IME
commits the text; active preedit/candidate composition remains unsupported.
The GUI's font and bidirectional text presentation are not certified as a
complete right-to-left editing experience.

| Area | Current status | Evidence required for promotion |
| --- | --- | --- |
| Core matching and config validation | Supported | Workspace unit tests and Clippy |
| CLI, JSON diagnostics, and config tooling | Supported | CLI and daemon smoke tests |
| systemd user services | Supported | `systemd-analyze verify` and installer test |
| wlroots virtual-keyboard output | Experimental | Target-compositor insertion test |
| libei/EIS output | Experimental | Portal consent, revocation, and reconnect tests; on-device tests for the `ei_keyboard`-only keysym-synthesis fallback (no `ei_text`), including non-US layouts |
| input-method-v2 capture | Experimental | Activation, focus, and Unicode tests |
| evdev capture (`--source=evdev --allow-evdev-sensitive-fields`) | Experimental, best-effort, explicit opt-in | Compositor-agnostic compatibility fallback (e.g. KWin); cannot detect password or other sensitive fields and must not be described as providing input-method privacy protections. Organizations that require this signal should enforce `safe_mode = true` and `require_sensitive_focus = true`. It also cannot make rapid replacement atomic under non-exclusive capture. See [`BACKENDS.md`](BACKENDS.md), [ADR 0001](adr/0001-evdev-best-effort-semantics.md), and [`SECURITY.md`](../SECURITY.md) |
| Global hotkeys | Experimental | Backend capture and action tests |
| Focused-window tracking (`app_filter`) | Supported by the KDE/KWin path; awaiting independent certification. Implemented on GNOME 45+ through the WayExpand GNOME Shell extension; tested in a headless GNOME Shell, awaiting real-session evidence | wlroots paths explicitly report application filters unavailable rather than guessing |
| Key pass-through | Experimental | libei-assisted lifecycle-aware press/release pass-through exists for input-method-v2; modifier chords, repetition, reconnect, and compositor/client behavior still require certification |
| Runtime keyboard-layout switching | Route-dependent | input-method-v2 and IBus expose layout-aware input paths; KDE evdev cross-checks KWin's active group against configured XKB settings; GNOME evdev reads Mutter's live input-source map, model, and options through the Shell extension (using Mutter's `pc105` model default on GNOME 45); Hyprland evdev validates compositor-reported per-device XKB settings and active group, refusing inconsistent keyboards or unknown layouts; Sway evdev matching remains unavailable because its input query omits the effective XKB settings. GNOME evdev layout tracking still requires live certification |
| External preedit/IME composition | **Not supported** | ⚠️ Affects CJK and active Fcitx/IBus/Rime composition (see below) |

## Desktop coverage

The shared backend route catalog also carries four desktop integration
profiles: `gnome-wayland`, `kde-wayland`, `sway`, and `hyprland`. Inspect them
with `wayexpand backend profile`; `wayexpand backend profile --current --json`
combines a detected profile with the live capability and policy recommendation.
Profiles list permissions, setup instructions, known limitations, verified
applications, and certification state. Their verified-application lists are
empty until the project accepts matching real-session evidence. A catalog
recommendation is guidance and does not assert current route availability.

No compositor is currently certified by automated end-to-end tests, and no
route is currently eligible to satisfy the production release gate. The release
workflow intentionally fails closed until a real-session route demonstrates
verified sensitive-field handling, atomic replacement, and the required
input/output contract. Before deploying desktop capture, test and record the
exact compositor and version:

The table below is generated from the certification target contract. Its
paths are test targets, not a guarantee that the path is available or works on
your session. Production certification changes only after reviewed
real-session evidence with `certification_scope: production` exists.

<!-- generated:desktop-certification-matrix:start -->
| Target | Desktop/session | Declared test paths | Window tracking | App filters | Sensitive fields observed | Production certification |
| --- | --- | --- | --- | --- | --- | --- |
| `kde` | KDE Plasma / KWin | ibus, evdev+libei | KWin application tracker | Available in declared path | Not observed | **Not certified** |
| `gnome` | GNOME Shell / Mutter | ibus, evdev+libei | WayExpand GNOME Shell extension (GNOME 45+) | Implemented; awaiting real-session evidence | Not observed | **Not certified** |
| `sway` | Sway / wlroots | evdev+wlroots, input-method-v2 | none | Unavailable | Not observed | **Not certified** |
| `hyprland` | Hyprland / wlroots | evdev+wlroots, input-method-v2 | none | Unavailable | Not observed | **Not certified** |
<!-- generated:desktop-certification-matrix:end -->

## Initial desktop and distribution validation plan

This execution matrix tracks the end-to-end checks needed for the launch
environments. “Not run” means there is no accepted real-session evidence yet;
protocol probes, CI, and the one-shot backend tests do not change that status.
Record exact distro, compositor, toolkit, route, and package versions with each
run. Promote only the tested combination.

| Environment | Priority | Install and activate | Plain text and Unicode | Deletion and replacement | Restart, upgrade, uninstall | Current evidence |
| --- | --- | --- | --- | --- | --- | --- |
| GNOME / Wayland | Critical | Not run | Not run | Not run | Not run | Not certified |
| KDE Plasma / Wayland | Critical | Not run | Not run | Not run | Not run | Not certified |
| Sway | High | Not run | Not run | Not run | Not run | Not certified |
| Hyprland | High | Not run | Not run | Not run | Not run | Not certified |
| Fedora GNOME | Critical | Not run | Not run | Not run | Not run | Not certified |
| Ubuntu GNOME | Critical | Not run | Not run | Not run | Not run | Not certified |
| KDE on Fedora | High | Not run | Not run | Not run | Not run | Not certified |

For evdev plus libei, test the two prerequisites independently with
`wayexpand backend test --capture` and `wayexpand backend test --injection`.
The first confirms device access and sanitized physical-input activity; the
second requires portal/EIS connection and operator confirmation of a harmless
marker in a controlled field. Passing both is useful route evidence, but does
not test their combined replacement behavior or certify the desktop route.
Evdev remains explicit opt-in because it has no sensitive-field signal.

<!-- generated:route-contract:start -->
| Route | Capture | Injection | Replacement assurance | Focus tracking | Sensitive fields (implementation) | Protocol signal | Compositor observation | Certification | Atomic replace | App identity | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `ibus` (IBus) | ibus | ibus | verified | unavailable | implemented | IBus content purpose | session-dependent | not certified | no | none | experimental |
| `kde-evdev-libei` (Maximum compatibility) | evdev | libei | unverified | available | not implemented | none | unavailable | not certified | no | kwin | experimental |
| `sway-evdev-wlroots` (Evdev + wlroots) | evdev | wlroots-virtual-keyboard | unverified | unavailable | not implemented | none | unavailable | not certified | no | none | experimental |
| `input-method-v2` (Input Method v2) | input-method-v2 | input-method-v2 | verified | unavailable | implemented | input-method-v2 content purpose | compositor-dependent | not certified | yes | none | experimental |
<!-- generated:route-contract:end -->

Replacement assurance is an explicit route property and a live status. The
route table's `verified` label means the protocol validates target text before
deletion; it does not by itself mean the erase and insert form one atomic
transaction. Automatic replacement requires an atomic contract by default.
Non-atomic routes, including routes that verify surrounding text, are blocked
by default until they pass the real-session target-integrity gate.
`compatibility_mode` remains an explicit experimental override for typed
triggers; it does not affect Quick Picker's safe clipboard fallback, which
copies globally applicable snippets for manual paste unless the daemon reports
an atomic route and exact target identity. App-filtered snippets are hidden
from Quick Picker because that manual path cannot retain verified application
identity. Enforced `require_verified_insert_target` policy blocks clipboard
fallback as well. With `compatibility_mode=off`, an unverified route refuses
before erasing and leaves the trigger available for recovery.
`unverified` is the baseline for keystroke routes. When
a compositor tracker confirms the exact focused window, live status refines that to
`focus-tracked-best-effort`: focus is checked, but the route still cannot
confirm the cursor or text being erased. If focus is unconfirmed, untracked
replacement is labeled `unverified` and requires explicit consent. With
`compatibility_mode` explicitly enabled, non-atomic replacement remains an
experimental compatibility mode and is not production-certified. Status says
`blocked` while the route is refused. See
[Insertion guarantees](../SECURITY.md#insertion-guarantees).

Keyboard-layout evidence is mandatory for certification: `us`, `de`, `fr`, an
AltGr-heavy layout, and a multi-layout switching setup. A US-only run is not
evidence for layout-independent text injection. Certification checks
dead-key/Compose input and expansion only after composition is committed; the
local keyboard-composition guard prevents a trigger from being matched while
those sequences are active. External active preedit is an explicit out-of-scope capability and does not become
supported through a passing certification. The machine-readable report lists
this boundary under `out_of_scope_capabilities`.

**Observed limitation: more than one configured layout on KWin's EIS.** With
two layouts configured (for example `us,de`), KWin 6.6.6 sends libei clients
a two-layout keymap but never reports which layout is active, not at connect
and not after a layout switch. This reproduced limitation applies to builds
that rely only on EIS modifier reports. The current daemon also consults the
compositor's layout provider and accepts its active group only when its
ordered layout groups match the EIS keymap. The EIS map is still used for
character synthesis; unrelated non-text keysym differences do not invalidate
the active-group signal. If the provider is unavailable or layout groups
differ, the fallback refuses with a clear diagnostic; WayExpand never selects
the first layout. The optional Fcitx5 bridge can provide a verified
Unicode direct-commit route when enabled and available. The original
limitation was observed in private headless KWin sessions on 2026-10-27
(commit `c8c27d86d632`, see
[`evidence/kwin-real-client-c8c27d86d632.txt`](evidence/kwin-real-client-c8c27d86d632.txt)),
not yet confirmed in a live Plasma session. The new provider-to-EIS keymap
agreement path has unit coverage but still needs repeatable live layout-switch
and physical-keyboard certification before this combination is certified.
Single-layout `us`, `de`, `fr`, and `us(altgr-intl)` sessions type correctly.

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
