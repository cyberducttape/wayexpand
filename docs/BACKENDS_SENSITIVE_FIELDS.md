# Sensitive Field Detection by Backend

This document describes password-field and sensitive-field protection capabilities across WayExpand backends.

## Overview

Sensitive-field detection allows WayExpand to automatically suspend text matching when you're typing in password fields or other sensitive inputs. This is a security feature that prevents credentials and private data from being accidentally expanded.

**Important:** Not all backends support this. Choose your backend knowing which protections are available.

## Backend Comparison

| Backend | Sensitive Detection | How It Works | When It Applies |
|---------|-------------------|--------------|-----------------|
| **input-method-v2** | ✅ Yes | Compositor sends content-type signal | Password, hidden-text, sensitive-data fields |
| **evdev** | ❌ No | Raw kernel input (no field signals) | Never — no way to detect |
| **libei** | ❌ No | Output-only (doesn't see input type) | Never — not an input source |
| **wlroots** | ❌ No | Output-only (doesn't see input type) | Never — not an input source |

## Input-Method-V2 (GNOME, Some Others)

**Detection Status:** ✅ Supported

When using `--source=input-method`:

- WayExpand receives content-type signals from the input method protocol
- These signals indicate the semantic purpose of the focused field:
  - `password` — Password input
  - `hidden-text` — Hidden text (PINs, tokens, etc.)
  - `sensitive-data` — Generic sensitive data
  - `unknown` — Treated as sensitive for safety
  - Other types — Allow expansion

- **Result:** Text matching is automatically suspended in all sensitive fields
- **User experience:** Typing in a password field works normally; expansion never occurs

**Tradeoff:** 
- ✅ Password protection is automatic
- ⚠️ Key pass-through is experimental (Escape/arrows/F-keys may not work)

## evdev (KDE Plasma, Sway, Hyprland)

**Detection Status:** ❌ Not Supported

When using `--source=evdev`:

- WayExpand reads keyboard events directly from `/dev/input/event*`
- These are raw kernel events with no semantic information
- There is **no way to detect which field has focus**
- The compositor provides no sensitive-field signal to the evdev reader

- **Result:** Text matching is NEVER suspended, even in password fields
- **User experience:** A typo in a password field could trigger an expansion

**Security implications:**

This is the documented tradeoff for using evdev. Before enabling evdev, you must:

1. Read [SECURITY.md](../SECURITY.md) completely
2. Understand that you're granting `input` group membership (raw keyboard access system-wide)
3. Accept that password fields are not protected

**When to use evdev despite this limitation:**

- You're on KDE Plasma/KWin where input-method-v2 isn't available
- You need reliable key pass-through for arrow keys, Escape, F-keys
- You trust your system and understand the security model
- You manually manage which applications you run (not a shared system)

**Mitigations:**

- Be cautious with trigger prefixes (`:` is safer than `;` for rare accidental matches)
- Use word-boundary matching to reduce false positives
- Don't store sensitive data (credentials, API keys) in expansion snippets
- Keep your snippet library security-reviewed
- Consider using a password manager instead of text expansions for credentials

## libei / EIS (Output Only)

**Detection Status:** ❌ Not Supported

libei and wlroots virtual-keyboard are output-only backends. They:

- Inject text into the active window
- Have no way to read which field is focused
- Cannot receive sensitive-field signals from the input method

**Result:** Rely entirely on your input source (input-method-v2 or evdev) for protection.

If using `--source=evdev --backend=libei`:
- No sensitive-field detection available (see evdev limitations above)

If using `--source=input-method --backend=libei`:
- Sensitive-field detection DOES work (input-method-v2 handles it)
- libei is just the injection mechanism

## Choosing Based on Your Needs

### "I need password protection"

Use `--source=input-method` on GNOME or compatible compositors.

```bash
systemctl --user enable --now wayexpand-input-method.service
```

**Trade-off:** Some keys may not pass through; see [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md).

### "I'm on KDE Plasma and need reliable keyboards"

Use `--source=evdev --backend=libei`. Accept no password protection.

```bash
sudo ./scripts/install-evdev-permissions.sh
systemctl --user enable --now wayexpand-evdev.service
```

Before enabling, read [SECURITY.md](../SECURITY.md) carefully. You're granting broad keyboard visibility.

### "I'm configuring for a team"

1. Document which backend each team member uses
2. Include security training about sensitive-field limitations
3. For sensitive systems, disable evdev by policy
4. For systems needing reliable keyboards, accept no password protection

## Testing Your Setup

To verify your backend's sensitive-field capability:

```bash
wayexpand doctor
```

Look for the "Sensitive-field detection" line:
- `AVAILABLE` — Your backend detects password fields
- `UNAVAILABLE` — Your backend does not (plan accordingly)

## Future Improvements

- [ ] Heuristic password-field detection based on field label/class (evdev)
- [ ] User-configurable sensitive-field indicators
- [ ] Per-snippet sensitive-field override
- [ ] Integration with system credential managers (read-only access)

---

**Summary:** Sensitive-field detection is automatic on input-method-v2, absent on evdev. Choose your backend knowing this tradeoff. Don't put credentials in snippets if using evdev.
