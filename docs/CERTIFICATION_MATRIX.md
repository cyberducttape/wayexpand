# WayExpand Compositor Certification Matrix

**Version:** 1.3.3
**Last Updated:** 2026-10-04
**Status:** No compositor route is production-certified

## Overview

This matrix records compatibility observations and certification evidence. A
manual test is not certification; the machine-readable artifact from
`wayexpand certify` is the single source of truth for production certification.

> **Important:** A route is certified only when a reviewed machine-readable artifact records real-session evidence and the live daemon satisfies the required capability contract. Manual observations and protocol availability do not certify a route.

Security-sensitive route claims have four separate meanings: **implemented by
WayExpand**, **advertised by the protocol**, **observed in the compositor
session**, and **certified on that compositor**. The generated tables below
keep those claims separate. An implemented content-purpose handler is not
evidence that a particular compositor delivered a reliable signal, and neither
is a certification claim.

## Quick Reference

This target summary is generated from the same machine-readable contract used
by the certification tools. Declared test paths are not per-session
availability claims, and no target is certified without reviewed evidence.

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

## Current evidence

There are no reviewed compositor certification artifacts in this repository.
The generated tables above are authoritative: every listed desktop and route
remains uncertified. Historical manual observations below do not establish
current compatibility because their session details and logs are not reviewed
certification artifacts. See [CERTIFICATION.md](CERTIFICATION.md) for the
evidence format and [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for current route
limitations.

## Historical manual observations (not certification evidence)

The following notes are retained only as historical context. They are not a
current support claim and must not be used to infer password-field protection.

**Test Date:** 2026-09-26  
**Configuration:**
- Desktop: KDE Plasma 6.6.x
- WayExpand: 1.3.3
- Capture: evdev (active-seat logind/uaccess ACL)
- Injection: libei
- Window Tracking: KWin D-Bus

**Test Results:**

| Test # | Scenario | Result | Notes |
|--------|----------|--------|-------|
| 1 | Basic expansion (`;hello` → "Hello World") | ✅ PASS | Clean insertion, no artifacts |
| 2 | Fast overlapping keys | ✅ PASS | Deduplication working correctly |
| 3 | Held key (prevent repeat expansion) | ✅ PASS | Single expansion despite held trigger |
| 4 | Password field protection | ⚠️ NOT AVAILABLE | Evdev has no password-field signal; expansions fire in all fields |
| 5 | Window-specific filtering (app_filter) | ✅ PASS | Trigger scoped correctly to Konsole |
| 6 | Multi-line replacement with newlines | ✅ PASS | All lines inserted, formatting preserved |
| 7 | Undo (Ctrl+Z) | ✅ PASS | Original trigger text restored |
| 8 | Clipboard interaction | ✅ PASS | Expansion content copyable to other apps |

**Known Limitations:**
- **Evdev capture cannot detect password fields.** Expansions fire in all input fields including passwords. Compositors using input-method-v2 for capture can provide sensitive-field signals; see [BACKENDS_SENSITIVE_FIELDS.md](BACKENDS_SENSITIVE_FIELDS.md).
- KWin D-Bus window tracking occasionally has ~100ms latency on window focus changes
- Input method selection can be finicky (workaround: use IBus explicitly)
- Evdev should use active-seat logind/uaccess ACLs; input-group membership is
  the broader legacy fallback.

**Recommendation:** Heavily manually tested, but not certified for production.
Evdev cannot provide password-field awareness; automated certification remains
pending. Users handling sensitive fields should prefer a backend with an
authoritative sensitive-field signal where available.

### ⚠️ GNOME 47.x - EXPERIMENTAL

**Configuration:**
- Desktop: GNOME 47.x
- Capture: input-method-v2 (no permissions needed)
- Injection: input-method-v2
- Window Tracking: N/A (text composition only)

**Known Limitations:**
- **No keyboard capture:** Function keys, arrow keys, Escape cannot be expanded. Use text-based alternatives.
- **No window tracking:** `app_filter`-scoped expansions fail closed; they are not applied globally.
- **Sensitive fields:** WayExpand implements input-method-v2 content-purpose
  handling, but GNOME/Mutter observation is compositor-dependent and has not
  been certified here. Do not treat the protocol signal as a production
  password-field protection guarantee until the GNOME password-field scenario
  has reviewed evidence.

**When to Use:**
- You primarily type text and don't need special keys
- You want zero permission configuration
- You do not need a certified password-field protection guarantee

**When NOT to Use:**
- You need arrow key expansion (`;up` → Up, etc.)
- You need function key expansion
- You need app-specific filtering

**Recommendation:** ⚠️ Experimental for v1.2. Suitable for text-focused workflows. Contributions welcome to improve window tracking.

### ⚠️ Sway 0.20.x - EXPERIMENTAL

**Configuration:**
- Compositor: Sway 0.20.x
- Capture: evdev
- Injection: wlroots virtual keyboard (experimental)
- Window Tracking: unavailable (not shipped)

**Known Limitations:**
- **Untested at scale:** Limited real-world usage feedback
- **Manual setup required:** Backend selection not automatic; requires explicit configuration
- **Portal reconnection edge cases:** libei socket reconnection not fully tested
- **No window tracking:** `app_filter`-scoped expansions fail closed on Sway.

**Recommendation:** ⚠️ Experimental. Likely compatible based on architecture, but limited validation. Contributions and testing welcome.

### ⚠️ Hyprland 0.45.x - EXPERIMENTAL

**Same as Sway 0.20.x**

In particular, focused-window tracking is not shipped on Hyprland, so
`app_filter`-scoped expansions fail closed rather than using the
`wlr-foreign-toplevel-management` protocol.

### ❌ X11 - UNSUPPORTED

WayExpand requires Wayland and will not be actively supported on X11. Reasons:
- X11's input model doesn't align with WayExpand's architecture
- No maintained Wayland migration path for X11-only desktops
- Modern desktops have transitioned to Wayland

**Workaround:** Use traditional text expansion tools (espanso, AutoKey) for X11.

## Testing Procedures

### Candidate production route: KDE Plasma + input-method-v2

The first production certification candidate is deliberately one narrow route:
KDE Plasma/KWin, `input-method-v2`, and the required US, DE, FR, AltGr, and
multi-layout profiles. This is a test target, not a certification claim; the
checked-in matrix remains **Not certified** until a real session produces a
reviewed artifact with sensitive-field, atomic-replacement, and full-Unicode
capabilities verified by the live daemon status.

Run the real client driver from a KDE session with GTK, Qt, a Chromium or
Electron client, a terminal, a password field, and an editor available:

```bash
scripts/run-certification-driver.sh \
  --driver /absolute/path/to/kde-golden-driver \
  --compositor kde --version "$(plasmashell --version | head -n1)" \
  --backend input-method-v2 \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk,qt,chromium,electron,terminal,password,editor \
  --output /tmp/wayexpand-kde-results.txt \
  --log-dir /tmp/wayexpand-kde-logs

scripts/certify-compositor.sh \
  --compositor kde --version "$(plasmashell --version | head -n1)" \
  --backend input-method-v2 \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk,qt,chromium,electron,terminal,password,editor \
  --results /tmp/wayexpand-kde-results.txt \
  --format json --output docs/certification/kde-<date>.json
```

The driver must exercise every scenario in
`tests/certification/compositor-matrix.json`, including pass-through for
Escape, arrows, function keys, modifiers, restart/reconnection, suspend and
resume, portal revocation, hotplug, Unicode/Compose, focus changes, password
fields, and GTK/Qt/Electron clients. A report is publishable only when every
in-scope scenario × layout × client cell is an observed `pass`, the doctor and
daemon probes identify the requested KDE session and backend, and a reviewer
checks the per-cell logs. `UNVERIFIED` is never a pass.

## Adding Your Testing Results

To report testing results for other compositors:

1. **Open a GitHub issue** with title: `Certification Test Results: [Compositor] [Version]`
2. **Include:**
   - Exact desktop/version
   - Configuration (capture, injection, window tracking)
   - Test date
   - Results for each test (PASS/FAIL)
   - Any discovered limitations
   - Environment details (hardware, other installed software)

3. **Example:**
   ```
   **Desktop:** Sway 0.20.0
   **Date:** 2026-10-05
   **Configuration:** evdev + libei
   **Results:**
   - Test 1 (Basic): PASS
   - Test 2 (Fast keys): FAIL - occasional duplication
   - Test 3 (Held key): PASS
   ...
   **Notes:** Duplication happens ~5% of the time with fast typing
   ```

## Certification Roadmap

The active release gate is defined in [CERTIFICATION.md](CERTIFICATION.md) and
`tests/certification/compositor-matrix.json`. No desktop has reviewed evidence
yet. Promote a route only after its required real-session scenario, layout, and
client cells pass and reviewers approve the resulting artifact.

## Important Notes

### What "Certified" Does NOT Mean
- ❌ Bug-free (no software is perfect)
- ❌ Every edge case tested (we test common scenarios)
- ❌ Performance guaranteed (latency depends on system)
- ❌ Supported forever (newer versions may have issues)

### What "Certified" DOES Mean
- Reviewed real-session evidence exists for every required matrix cell.
- The live route reports every mandatory safety and injection capability.
- The evidence identifies the compositor version, route, layouts, clients, and observed results.

### Version-Specific Support

Use the generated Quick Reference table above for current desktop and route
status. This document does not maintain separate version-specific support
claims.

## Enterprise Deployment

The organization policy file controls enforcement; it does not select the
capture, injection, or window-tracking route. Those are runtime/backend
selection concerns and must be verified separately with the certification
artifact. The following is an executable policy example:

<!-- executable-toml: config -->
```toml
[organization]
safe_mode = true
require_atomic_replace = true
allowed_backends = ["libei"]
audit_prefix = "wayexpand-enterprise"
```

`allowed_backends = ["libei"]` permits only the libei injector once the
selected input source has passed its runtime checks. It does not certify KDE,
KWin tracking, password-field handling, or any client/toolkit; those claims
require a checked-in evidence artifact.

### Text-only fallback policy

If the deployment intentionally permits the input-method-v2 text-only route,
use a separate policy such as:

<!-- executable-toml: config -->
```toml
[organization]
safe_mode = true
allowed_backends = ["input-method-v2"]
require_sensitive_focus = true
audit_prefix = "wayexpand-text-only"
```

This policy still does not make the route certified. Active IME/preedit
composition remains outside the certification scope.

## Getting Help

- **KDE Plasma:** See [docs/SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for KDE-specific setup
- **GNOME:** See [docs/CAPTURE_BACKEND_TRADEOFFS.md](CAPTURE_BACKEND_TRADEOFFS.md) for input-method-v2 guide
- **Sway/Hyprland:** See [docs/BACKENDS.md](BACKENDS.md) for manual configuration
- **Not certified?** Open an issue with testing results—we'd love to expand certification coverage

## See Also

- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — Current support status
- [CAPTURE_BACKEND_TRADEOFFS.md](CAPTURE_BACKEND_TRADEOFFS.md) — Input method guide
- [BACKENDS.md](BACKENDS.md) — Manual backend configuration
- [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) — Full test procedures
