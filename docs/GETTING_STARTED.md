# Getting Started with WayExpand

**Navigation:** [Home](../README.md) > **Getting Started**

---

Welcome! This guide will help you get up and running based on your desktop environment.

## Five-Minute Quickstart

This is the shortest path from an installed binary to a working snippet.

### 1. Check the session

```sh
wayexpand doctor
wayexpand explain-backend
```

Use the backend combination that `doctor` prints as usable. Backend support is
compositor-dependent; an `Implemented` line means the code exists, not that
your current session has advertised the protocol.

### 2. Open the terminal editor

```sh
wayexpand-ui
```

Press `n`, enter a trigger such as `;;hello`, enter `Hello, world!`, and press
`Enter` to save. Use `/` to search, `e` to edit the selected snippet, `Space`
to enable/disable it, and `q` to quit. The same file can be edited with the
graphical editor:

```sh
wayexpand-gui
```

The default file is `~/.config/wayexpand/expansions.toml` (or the path set by
`WAYEXPAND_CONFIG`). Both editors save atomically and validate before writing.

### 3. Start the route selected for your session

Automatic mode intentionally leaves raw evdev disabled, even when
`/dev/input` is readable. If you choose to acknowledge global keyboard capture,
install the evdev permission rule and start the explicit service:

```sh
sudo ./scripts/install-evdev-permissions.sh
systemctl --user enable --now wayexpand-evdev.service
wayexpand status --json
```

The status should report `state=connected`. If you do not acknowledge evdev,
the automatic fallback is stdin and is intended for harnesses, not normal
desktop capture. Input-method-v2 is an explicit experimental opt-in; read its
key pass-through warning before starting
`wayexpand-input-method.service`. For either route, read the security
tradeoffs in [SECURITY.md](../SECURITY.md).

### 4. Test without typing into an application

```sh
wayexpand test ';;hello'
```

If this prints `Hello, world!`, type `;;hello` into a plain text field. If it
prints `no expansion matched`, inspect the trigger with `wayexpand list` and
`wayexpand validate` before debugging the backend.

### 5. Add an app-filtered snippet (optional)

Add this in the TUI/GUI or to the TOML file:

```toml
[[expansion]]
trigger = ";;ticket"
replacement = "Investigating this now."
description = "Support response for the ticketing app"
app_filter = ["com.example.TicketApp"]
```

App filters only work when the current compositor has a working window tracker.
They fail closed when tracking is unavailable. Verify the exact application ID
with the desktop's window-inspection tools and confirm support in
[SUPPORT_MATRIX.md](SUPPORT_MATRIX.md); use a global snippet if the tracker is
experimental or unavailable.

For the output-driven recovery flow, go directly to the
[Troubleshooting Checklist](TROUBLESHOOTING_CHECKLIST.md).

### Architecture note: aarch64

The release archive currently has a pre-built Linux binary only for `x86_64`.
There is no official aarch64 binary yet. On aarch64, follow the
[source-build instructions in the packaging guide](PACKAGING.md#aarch64) and
run `wayexpand doctor` after building. Fedora Copr is also not published yet;
Fedora users should build from source or the RPM spec for now.

## Which Desktop Are You Using?

### KDE Plasma Wayland (KWin) — implementation candidate

This is the most complete implementation path, but it still requires live
session testing and is not certified by the current support matrix. Start
here only after reviewing the backend tradeoffs:

```bash
# Install
sudo apt install wayexpand  # Ubuntu/Debian
# or: build the repository PKGBUILD with `makepkg -si` # Arch (preview)

# Start the explicit evdev route (after reviewing its raw-input tradeoff)
wayexpand-daemon --source=evdev --backend=libei ~/.config/wayexpand/expansions.toml

# Launch the GUI
wayexpand-gui
```

**Current status:** Use `wayexpand doctor` to confirm the available path. The
KWin window tracker is implemented but still awaiting broader independent
certification; app-restricted snippets should not be treated as universally
verified. KWin 6.6 has live-session observations in the backend notes, but
newer KWin releases must still be checked with `doctor` and certification
evidence instead of assumed compatible from the version number.

**Next:** See [README.md](../README.md) for examples. The GUI walks you through creating your first snippet.

---

### Sway / Hyprland / river — experimental

Wlroots-based compositor support has active development in progress. Window
tracking is not currently shipped.

**Current status:**
- Text capture and injection paths are implemented (via input-method-v2 or
  evdev), but have not been certified on these compositors
- ⏳ Window tracking (app_filter) is not shipped on wlroots; the integration remains future experimental work
- ⚠️ Password field detection requires input-method-v2

**Getting started:**

```bash
wayexpand-daemon --source=evdev --backend=libei ~/.config/wayexpand/expansions.toml
wayexpand doctor  # Shows what your session can use
```

**Known limitations:**
- Automatic mode leaves evdev disabled even when `/dev/input` is readable
- Input-method-v2 is explicit/experimental: Escape, arrow keys, and F-keys may not pass through
- Evdev requires `input` group membership, has no password-field signal, and is best-effort under rapid typing

**Want to help test?**
- Run `wayexpand doctor` and share the output on our [GitHub issues](https://github.com/itchyitchy123/wayexpand/issues)
- Test snippets and report what works/breaks
- See [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) for detailed testing

---

### GNOME Shell Wayland — limited/uncertified

Limited support due to GNOME's design (no window tracking protocol).

**What works:**
- Text capture via input-method-v2 when explicitly enabled (but no window
  tracking); this path remains uncertified

**What doesn't work:**
- ❌ Window-specific snippets (app_filter)
- ❌ Password field protection

**Recommendation:**
- Use **global hotkeys** instead of trigger-based expansion, or explicitly opt into input-method-v2 after testing key pass-through
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
- [TROUBLESHOOTING_CHECKLIST.md](TROUBLESHOOTING_CHECKLIST.md) - doctor output → fix
- [PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md) - matcher and large-library tuning
- [OPERATIONS.md](OPERATIONS.md) - Systemd management and troubleshooting
- [BACKENDS.md](BACKENDS.md) - Backend protocols and compatibility

---

## Next Steps

- **Learn the config format:** [Configuration Limits](CONFIGURATION_LIMITS.md) and [Compatibility](COMPATIBILITY.md)
- **Understand the architecture:** [README.md](../README.md#architecture)
- **Report bugs:** See [SECURITY.md](../SECURITY.md#reporting-a-vulnerability) for security issues, or open a GitHub issue for bugs

Welcome to WayExpand! 🎉
