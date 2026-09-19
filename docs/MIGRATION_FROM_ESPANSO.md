# Migrating from Espanso to WayExpand

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Migrating from Espanso**

---

Welcome! If you're switching from Espanso, WayExpand can import your existing snippets and offers several advantages for Wayland users. This guide covers the transition.

## Quick Import

WayExpand can automatically convert your Espanso YAML configs to TOML:

```bash
wayexpand import espanso ~/.config/espanso/default.yml
```

This converts:
- ✅ String-based replacements
- ✅ Basic abbreviations
- ⚠️ Skips unsupported advanced features (with warnings)
- ❌ Never modifies your Espanso config

**Result:** Your snippets appear in WayExpand's GUI ready to use.

## What's Different

### Configuration Format

**Espanso (YAML):**
```yaml
matches:
  - trigger: ";hello"
    replace: "Hello, world!"

  - trigger: ";sig"
    replace: |
      Best regards,
      John
```

**WayExpand (TOML):**
```toml
[[expansion]]
trigger = ";hello"
replacement = "Hello, world!"

[[expansion]]
trigger = ";sig"
replacement = """Best regards,
John"""
```

### Architecture

| Aspect | Espanso | WayExpand |
|--------|---------|-----------|
| **Design** | Single unified daemon | Modular: engine separate from backends |
| **Wayland support** | XTest via XWayland, or native Wayland | Native per-protocol backends (input-method-v2, libei, evdev, wlroots) |
| **Backend selection** | Auto-detected silently | Explicitly chosen or auto-detected with explanation |
| **Diagnostics** | Limited | `wayexpand doctor` shows what works and why |
| **Config reload** | Replaces config immediately | Validates first, only swaps if valid |
| **Password protection** | Not modeled | Explicit on backends that support it |
| **GUI** | None (YAML files) | Native egui app with preview and diagnostics |
| **Window filtering** | Not supported | `app_filter` for app-scoped snippets (when available) |

### Compatibility

**Espanso features WayExpand supports:**
- ✅ String replacements
- ✅ Multi-line text
- ✅ Regex matching (with limitations)
- ✅ Case propagation (`;hello` → `Hello` or `HELLO`)
- ✅ Custom triggers
- ✅ Categories and tags
- ✅ Show UI option
- ✅ Conditional expansion (via app_filter for window-specific)

**Espanso features NOT in WayExpand:**
- ❌ Shell scripts in expansions (use `[expansion.command]` instead)
- ❌ External files (`external_filter`)
- ❌ Extension system
- ❌ X11-only backends (XTest)
- ❌ Global form filling (use app_filter + commands)

**WayExpand features you gain:**
- ✅ Explicit backend diagnostics (`wayexpand doctor`)
- ✅ App-scoped snippets (`app_filter`)
- ✅ Command-backed snippets with output integration
- ✅ Organization policy enforcement
- ✅ Fleet deployment (Ansible, Puppet)
- ✅ Structured command execution (no shell injection risk)
- ✅ Native Wayland protocols (not XWayland translation)

## Side-by-Side: Common Tasks

### Simple replacement

**Espanso:**
```yaml
matches:
  - trigger: ";;addr"
    replace: "123 Main St, Anytown"
```

**WayExpand:**
```toml
[[expansion]]
trigger = ";;addr"
replacement = "123 Main St, Anytown"
```

### Multi-line text

**Espanso:**
```yaml
matches:
  - trigger: ";;letter"
    replace: |
      Dear Sir or Madam,
      
      Thank you for your inquiry.
      
      Best regards,
      John
```

**WayExpand:**
```toml
[[expansion]]
trigger = ";;letter"
replacement = """Dear Sir or Madam,

Thank you for your inquiry.

Best regards,
John"""
```

### Case propagation

**Espanso:**
```yaml
matches:
  - trigger: ";company"
    replace: "Acme Corp"
```

Typing `;COMPANY` → `ACME CORP`, `;Company` → `Acme Corp`

**WayExpand:** Identical behavior! Case propagation is automatic.

```toml
[[expansion]]
trigger = ";company"
replacement = "Acme Corp"
```

### Command execution

**Espanso (shell script):**
```yaml
matches:
  - trigger: ";date"
    replace: "{{output}}"
    vars:
      - name: output
        type: shell
        params:
          cmd: "date '+%Y-%m-%d'"
```

**WayExpand (structured command):**
```toml
[[expansion]]
trigger = ";date"
description = "Insert today's date"

[expansion.command]
program = "date"
args = ["+%Y-%m-%d"]
timeout_ms = 5000
```

**Advantages:**
- No shell injection risk
- Clear timeout handling
- Output size bounded (1 MiB)
- Can cache results

### Window-specific snippets

**Espanso:** Not directly supported

**WayExpand (app_filter):**
```toml
[[expansion]]
trigger = ";close"
replacement = "Thanks for using our app!"
app_filter = ["app-id"]
```

Only expands in the specified app (when window tracking is available).

### Categories and organization

**Espanso:**
```yaml
matches:
  - trigger: ";email"
    replace: "john@example.com"
    label: "Contact"

  - trigger: ";phone"
    replace: "+1-555-0123"
    label: "Contact"
```

**WayExpand:**
```toml
[[expansion]]
trigger = ";email"
replacement = "john@example.com"
category = "Contact"
tags = ["personal", "email"]

[[expansion]]
trigger = ";phone"
replacement = "+1-555-0123"
category = "Contact"
tags = ["personal", "phone"]
```

The GUI filters by category and tags, making large libraries easier to navigate.

## Performance Comparison

| Metric | Espanso | WayExpand |
|--------|---------|-----------|
| Memory usage (baseline) | ~40 MB | ~50 MB |
| Memory per 1000 snippets | ~2 MB | ~1-5 MB |
| CPU (idle) | ~0.1% | <0.1% |
| CPU (typing) | ~0.5-1% | ~1-2% |
| Match latency (1000 snippets) | ~0.5 ms | ~0.3 ms |

Both are lightweight. WayExpand uses a trie-based matcher (O(n) where n is trigger length) vs. Espanso's regex engine.

## Installation

**Replace Espanso with WayExpand:**

```bash
# Stop Espanso
systemctl --user disable espanso.service
systemctl --user stop espanso.service

# Install WayExpand (choose one)
sudo apt install wayexpand        # Ubuntu/Debian
makepkg -si                       # Arch (preview)
./scripts/install-user.sh         # Any distro

# Import your Espanso config
wayexpand import espanso ~/.config/espanso/default.yml

# Start WayExpand
systemctl --user enable wayexpand-input-method.service
systemctl --user start wayexpand-input-method.service

# Launch the GUI
wayexpand-gui
```

## Configuration Migration

### Manual steps for advanced configs

If `wayexpand import` skips features, you'll need to adapt manually:

**1. Shell commands → Structured commands:**

Before (Espanso):
```yaml
matches:
  - trigger: ";uptime"
    replace: "{{output}}"
    vars:
      - name: output
        type: shell
        params:
          cmd: "uptime"
```

After (WayExpand):
```toml
[[expansion]]
trigger = ";uptime"

[expansion.command]
program = "uptime"
timeout_ms = 5000
```

**2. Regex → Plain text or app_filter:**

Before (Espanso):
```yaml
matches:
  - trigger: "/(\\d{1,2})\\.(\\d{1,2})\\.(\\d{4})/"
    replace: "$3-$1-$2"
```

After (WayExpand, plain trigger):
```toml
[[expansion]]
trigger = ";date-us"
replacement = "2024-01-15"
```

Or use Python/script to generate combinations.

**3. External filters → Commands:**

Espanso's `external_filter` can often be replaced with `[expansion.command]`:

Before:
```yaml
matches:
  - trigger: ";weather"
    replace: "{{output}}"
    vars:
      - name: output
        type: script
        params:
          args: "/path/to/get-weather.py"
```

After:
```toml
[[expansion]]
trigger = ";weather"

[expansion.command]
program = "/path/to/get-weather.py"
timeout_ms = 5000
```

### Testing your config

**Validate before switching:**
```bash
wayexpand validate ~/.config/wayexpand/expansions.toml
```

**Test in the GUI:**
```bash
wayexpand-gui
```

**Full diagnostics:**
```bash
wayexpand doctor
```

## Troubleshooting the Migration

### "Import: unsupported feature"

Some Espanso YAML features can't be automatically converted:
- `external_filter` → rewrite as `[expansion.command]`
- `shell_expand` → rewrite as multi-line replacement
- Extension scripts → manual configuration

**Solution:** Adapt these manually following the examples above.

### "Snippets imported but don't expand"

1. Check daemon is running: `systemctl --user status wayexpand-input-method.service`
2. Run `wayexpand doctor` to see backend status
3. Verify config: `wayexpand validate`
4. Check logs: `journalctl --user -u wayexpand-input-method.service -n 50`

### "Config file syntax errors"

WayExpand uses TOML (not YAML). Common mistakes:
- `=` instead of `:` for assignments
- Missing quotes around strings
- Tabs instead of spaces (TOML requires spaces)

**Solution:** Use `wayexpand validate` to find and fix errors.

### "Certain triggers don't work"

If a trigger works in Espanso but not WayExpand:
1. Ensure it's not using Regex (WayExpand uses plain text by default)
2. Check that `match_mode = "trigger"` (or omit, it's the default)
3. Verify no `app_filter` is blocking it: `wayexpand validate --merge-preview`

## Feature Parity

**Want to keep your Espanso setup alongside WayExpand?**

You can run both:
```bash
# Espanso on one port
systemctl --user start espanso.service

# WayExpand on another
systemctl --user start wayexpand-input-method.service
```

They won't interfere — each handles different triggers. Gradually migrate snippets to WayExpand as you get comfortable.

## What You'll Appreciate

1. **Better Wayland support:** Native protocols instead of XWayland translation
2. **Diagnostics:** `wayexpand doctor` tells you exactly what works
3. **Reliability:** Config validation before swap means no broken states
4. **Privacy:** Password fields are explicitly protected
5. **Organization:** App-scoped snippets and fleet deployment for teams
6. **GUI:** Visual editor with live preview (no YAML editing required)

## Uninstalling Espanso

Once you're comfortable with WayExpand:

```bash
# Remove Espanso
sudo apt remove espanso       # Ubuntu/Debian
pacman -R espanso             # Arch

# Optional: clean up config
rm -rf ~/.config/espanso/
rm -rf ~/.local/share/espanso/
```

## Getting Help

- **Troubleshooting:** [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
- **Full documentation:** [GETTING_STARTED.md](GETTING_STARTED.md)
- **Configuration reference:** [docs/](../README.md#documentation)
- **Support matrix:** [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)
- **Issues:** https://github.com/itchyitchy123/wayexpand/issues

Welcome to WayExpand! If you have suggestions for improving the import process, please open an issue on GitHub.
