# WayExpand Compositor Certification Matrix

**Version:** 1.2.0  
**Last Updated:** 2026-09-26  
**Status:** Pre-release certification (manual testing phase)

## Overview

This matrix records compatibility observations and certification evidence. A
manual test is not certification; the machine-readable artifact from
`wayexpand certify` is the single source of truth for production certification.

> **Important:** "Certified" means thoroughly tested with published procedures. "Experimental" means limited testing or known limitations. "Unsupported" means no active support—may work, but not recommended for production.

## Quick Reference

| Desktop | Version | Capture | Injection | Window Track | Status | Tested | Notes |
|---------|---------|---------|-----------|--------------|--------|--------|-------|
| **KDE Plasma** | 6.6.x | evdev | libei | KWin D-Bus | Manually tested; not certified | 2026-09 | evdev has no password-field awareness; automated certification pending |
| **KDE Plasma** | 6.5.x | evdev | libei | KWin D-Bus | Experimental | pending | Likely compatible |
| **KDE Plasma** | 6.7.x | evdev | libei | KWin D-Bus | Experimental | pending | Pre-release testing welcome |
| **GNOME** | 47.x | input-method-v2 | input-method-v2 | N/A | Experimental | pending | Full text support, no keyboard capture |
| **GNOME** | 46.x | input-method-v2 | input-method-v2 | N/A | Experimental | pending | Full text support, no keyboard capture |
| **Sway** | 0.20.x | evdev/libei | libei | wlr-foreign-toplevel | Experimental | pending | Requires manual setup |
| **Sway** | 0.19.x | evdev/libei | libei | wlr-foreign-toplevel | Experimental | pending | Requires manual setup |
| **Hyprland** | 0.45.x | evdev/libei | libei | wlr-foreign-toplevel | Experimental | pending | Requires manual setup |
| **Hyprland** | 0.40.x | evdev/libei | libei | wlr-foreign-toplevel | Experimental | pending | Requires manual setup |
| **river** | 0.4.x | evdev/libei | libei | wlr-foreign-toplevel | Experimental | pending | Minimal testing |
| **X11** | any | evdev | clipboard | N/A | Unsupported | N/A | Legacy; not supported for v1.2+ |

## Detailed Certification Results

### ⚠️ KDE Plasma 6.6.x - MANUALLY TESTED, NOT CERTIFIED

**Test Date:** 2026-09-26  
**Configuration:**
- Desktop: KDE Plasma 6.6.x
- WayExpand: 1.2.0
- Capture: evdev (input group)
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
- Evdev requires input group membership

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
- **No window-specific filtering:** `app_filter` does not work; expansions apply globally to all apps
- **No sensitive field detection:** Relies on GNOME's text input filtering (usually works, but not guaranteed)

**When to Use:**
- You primarily type text and don't need special keys
- You want zero permission configuration
- You need password field protection

**When NOT to Use:**
- You need arrow key expansion (`;up` → Up, etc.)
- You need function key expansion
- You need app-specific filtering

**Recommendation:** ⚠️ Experimental for v1.2. Suitable for text-focused workflows. Contributions welcome to improve window tracking.

### ⚠️ Sway 0.20.x - EXPERIMENTAL

**Configuration:**
- Compositor: Sway 0.20.x
- Capture: evdev + libei
- Injection: libei
- Window Tracking: wlr-foreign-toplevel protocol

**Known Limitations:**
- **Untested at scale:** Limited real-world usage feedback
- **Manual setup required:** Backend selection not automatic; requires explicit configuration
- **Portal reconnection edge cases:** libei socket reconnection not fully tested

**Recommendation:** ⚠️ Experimental. Likely compatible based on architecture, but limited validation. Contributions and testing welcome.

### ⚠️ Hyprland 0.45.x - EXPERIMENTAL

**Same as Sway 0.20.x**

### ❌ X11 - UNSUPPORTED

WayExpand requires Wayland and will not be actively supported on X11. Reasons:
- X11's input model doesn't align with WayExpand's architecture
- No maintained Wayland migration path for X11-only desktops
- Modern desktops have transitioned to Wayland

**Workaround:** Use traditional text expansion tools (espanso, AutoKey) for X11.

## Testing Procedures

### For KDE Plasma 6.6.x manual testing

The following reproduces the manual compatibility observations. It is not a
certification procedure: evdev cannot detect password fields, and only a
machine-readable `wayexpand certify` artifact can establish certification.

1. **Setup:**
   ```bash
   # Install WayExpand 1.2.0
   sudo apt install wayexpand=1.2.0-*
   
   # Verify evdev access
   groups $USER | grep input
   # If not present: sudo usermod -aG input $USER && logout/login
   ```

2. **Create a temporary test configuration**
   ```toml
   # Save as ~/.config/wayexpand-certification-test.toml with mode 0600.
   [[expansion]]
   trigger = ";hello"
   replacement = "Hello World"

   [[expansion]]
   trigger = ";a"
   replacement = "AAA"

   [[expansion]]
   trigger = ";pw"
   replacement = "MyPassword123"

   [[expansion]]
   trigger = ";konsole"
   replacement = "KonsoleTest"
   app_filter = ["konsole"]
   ```
   Validate it with:
   ```bash
   chmod 600 ~/.config/wayexpand-certification-test.toml
   wayexpand validate ~/.config/wayexpand-certification-test.toml
   ```

3. **Test 1: Basic Expansion**
   ```bash
   wayexpand doctor "$HOME/.config/wayexpand-certification-test.toml"
   # Open Kate or any text editor
   # Type: ;hello
   # Press: Space
   # Expected: "Hello World" inserted
   ```

4. **Test 2: Fast Overlapping Keys**
   ```bash
   # In text editor, type rapidly: ;;h e l l o
   # Expected: Only "Hello World" inserted once, not duplicated
   ```

5. **Test 3: Held Key**
   ```bash
   # Press and hold ; for 2 seconds, then type 'a' while held
   # Expected: Exactly one "AAA" inserted, not repeated
   ```

6. **Test 4: Password Fields**
   ```bash
   # Open KDE Wallet or any password field
   # Type: ;pw
   # Press: Space
   # Evdev limitation: do not expect blocking; expansion may be inserted.
   ```

7. **Test 5: App Filtering**
   ```bash
   # In Konsole: type ;konsole → Space
   # Expected: "KonsoleTest" inserted
   
   # In Kate: type ;konsole → Space
   # Expected: NOT inserted (blocked by app_filter)
   ```

7. **Test 6-8:** See full procedures in [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md)

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

### v1.2.0 (Current)
- ⚠️ KDE Plasma 6.6.x heavily manually tested; not certified
- ✅ Documentation records the evdev password-field limitation
- Automated certification remains pending

### v1.2.1+
- [ ] Test and publish GNOME 47.x results (if time permits)
- [ ] Gather community testing for Sway, Hyprland
- [ ] Update matrix based on real-world feedback

### v1.3.0+
- [ ] Build CI automation for E2E testing
- [ ] Auto-test against multiple compositor versions
- [ ] Generate certification matrix from CI results
- [ ] Add regression tests for discovered bugs

## Important Notes

### What "Certified" Does NOT Mean
- ❌ Bug-free (no software is perfect)
- ❌ Every edge case tested (we test common scenarios)
- ❌ Performance guaranteed (latency depends on system)
- ❌ Supported forever (newer versions may have issues)

### What "Certified" DOES Mean
- ✅ Explicitly tested with published procedures
- ✅ Results reproducible by others
- ✅ Known limitations documented
- ✅ Production-ready based on evidence
- ✅ Professional quality assurance

### Version-Specific Support

**WayExpand 1.2.0 will support:**
- ⚠️ KDE Plasma 6.6.x (heavily manually tested; not certified)
- ✅ KDE Plasma 6.5.x, 6.7.x (expected to work, experimental)
- ✅ GNOME 47.x, 46.x (input-method-v2 path, experimental)
- ✅ Sway, Hyprland, river (experimental, contributions welcome)
- ❌ X11 (unsupported, use traditional text expansion tools)

## Enterprise Deployment

### Recommended Configuration
```toml
[organization]
# No certified configuration is currently published; review the machine-readable artifact.
capture_method = "evdev"
injection_method = "libei"
window_tracking = "kwin-d-bus"

# Enforce certified mode
require_certified_mode = true
```

### Safe Fallback
```toml
[organization]
# If KDE Plasma not available, fall back to input-method-v2
primary_mode = "kde-evdev-libei"
fallback_mode = "input-method-v2-safe"  # No keyboard capture, text only
```

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
