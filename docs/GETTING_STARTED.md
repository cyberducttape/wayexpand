# Getting Started with WayExpand

**Navigation:** [Home](../README.md) > **Getting Started**

---

Welcome! This guide will help you get up and running based on your desktop environment.

## Which Desktop Are You Using?

### KDE Plasma Wayland (KWin 6.6+) ✅

You have the best support. Start here:

```bash
# Install
sudo apt install wayexpand  # Ubuntu/Debian
# or: build the repository PKGBUILD with `makepkg -si` # Arch (preview)

# Start the daemon directly (or enable the installed user service)
wayexpand-daemon --source=input-method ~/.config/wayexpand/expansions.toml

# Launch the GUI
wayexpand-gui
```

**Current status:** Use `wayexpand doctor` to confirm the available path. The
KWin window tracker is implemented but still awaiting broader independent
certification; app-restricted snippets should not be treated as universally
verified.

**Next:** See [README.md](../README.md) for examples. The GUI walks you through creating your first snippet.

---

### Sway / Hyprland / river ⚠️

Wlroots-based compositors have active development in progress. Window tracking is nearly ready.

**Current status:**
- ✅ Text capture and injection work (via input-method-v2 or evdev)
- ⏳ Window tracking (app_filter) remains a scaffold awaiting protocol-flow and compositor certification
- ⚠️ Password field detection requires input-method-v2

**Getting started:**

```bash
wayexpand-daemon --source=input-method ~/.config/wayexpand/expansions.toml
wayexpand doctor  # Shows what your session can use
```

**Known limitations:**
- Escape, arrow keys, and F-keys may not work with input-method-v2
- If that's a blocker, use evdev: `--source=evdev` (requires `input` group)

**Want to help test?**
- Run `wayexpand doctor` and share the output on our [GitHub issues](https://github.com/itchyitchy123/wayexpand/issues)
- Test snippets and report what works/breaks
- See [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) for detailed testing

---

### GNOME Shell Wayland ❌

Limited support due to GNOME's design (no window tracking protocol).

**What works:**
- ✅ Global hotkeys (text expansion via custom keyboard shortcuts)
- ⚠️ Text capture via input-method-v2 (but no password field detection)

**What doesn't work:**
- ❌ Window-specific snippets (app_filter)
- ❌ Password field protection

**Recommendation:**
- Use **global hotkeys** instead of trigger-based expansion
- See [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for compatibility status

---

## Common First Steps

Regardless of your compositor:

### 1. Verify Your Setup

```bash
wayexpand doctor
```

This shows you what capture, output, and window-tracking backends are available in your session. **Always run this before troubleshooting.**

### 2. Create Your First Snippet

**Via GUI:**
```bash
wayexpand-gui
```
Click "Add snippet" and enter:
- **Trigger:** `;;hello`
- **Replacement:** `Hello, world!`

Click Save, then type `;;hello` in any text field.

**Via CLI:**
```bash
$EDITOR ~/.config/wayexpand/expansions.toml
```

Add to the TOML:
```toml
[[expansion]]
trigger = ";;hello"
replacement = "Hello, world!"
```

Save, then the daemon reloads automatically.

### 3. Common Patterns

**Email signature:**
```toml
[[expansion]]
trigger = ";;sig"
replacement = """Best,
Your Name"""
```

**Date (auto-generated):**
```toml
[[expansion]]
trigger = ";;date"
replacement = "{{date}}"  # Will be today's date
```

**Code snippet:**
```toml
[[expansion]]
trigger = ";;func"
replacement = """def my_function(x):
    return x * 2"""
```

See [COMPATIBILITY.md](COMPATIBILITY.md) for the configuration format, or view the [archived configuration reference](archive/wiki/Configuration.md) for variable examples.

---

## Troubleshooting

**Snippets aren't expanding?**
1. Run `wayexpand doctor` to check backend status
2. Verify the daemon is running: `systemctl --user status wayexpand-input-method.service`
3. Check logs: `journalctl --user -u wayexpand-input-method.service -n 20`
4. Try a simple test: `;;hello` should become `Hello, world!`

**Password field protection not working?**
- This requires input-method-v2 backend (shows in `wayexpand doctor`)
- evdev capture (`--source=evdev`) has no password detection
- GNOME Shell doesn't report password fields at all

**Keys are lost (Escape, arrows, F-keys)?**
- This is input-method-v2's limitation on some compositors
- Use evdev as fallback: `wayexpand-daemon --source=evdev ~/.config/wayexpand/expansions.toml`
- Remember: evdev requires `input` group and has no password protection

**See Also:**
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) - What's verified vs. experimental
- [OPERATIONS.md](OPERATIONS.md) - Systemd management and troubleshooting
- [BACKENDS.md](BACKENDS.md) - Backend protocols and compatibility

---

## Next Steps

- **Learn the config format:** [Configuration Limits](CONFIGURATION_LIMITS.md) and [Compatibility](COMPATIBILITY.md)
- **Understand the architecture:** [README.md](../README.md#architecture)
- **Report bugs:** See [SECURITY.md](../SECURITY.md#reporting-a-vulnerability) for security issues, or open a GitHub issue for bugs

Welcome to WayExpand! 🎉
