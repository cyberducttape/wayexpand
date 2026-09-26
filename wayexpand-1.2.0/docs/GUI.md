# WayExpand GUI

This guide covers the settings frontends, themes, languages, retro fonts, and GUI performance behavior.

## WayExpand settings UI

WayExpand ships two settings frontends: `wayexpand-ui` is dependency-light and
terminal-native, while `wayexpand-gui` is a graphical Wayland-capable editor.
Both use the same core model and atomic-save path.

Start it with:

```sh
wayexpand-ui
wayexpand-ui /path/to/expansions.toml
wayexpand-gui /path/to/expansions.toml
```

If the requested configuration does not exist, the graphical frontend creates
an empty private configuration and opens the editor. Existing malformed,
insecure, or unreadable files are never replaced automatically; they remain a
visible startup error for safe recovery.

Controls:

- `/` starts snippet search
- `j`/`k` or the arrow keys select a snippet
- `n` creates a snippet, prompting for trigger and replacement
- `e` edits the selected replacement
- `D` edits the selected description
- `t` edits comma-separated tags
- `Ctrl-U` clears the active prompt before entering replacement text
- `m` toggles immediate and Unicode-aware word-boundary matching
- `E` opens the selected replacement in `$VISUAL` or `$EDITOR` for multiline editing
- `d` then `d` again deletes the selected snippet (Esc cancels)
- `u` undoes the last saved change
- `Space` toggles the selected snippet and saves atomically
- `r` reloads the configuration from disk
- `p` pauses or resumes the running daemon
- `q`, `Esc`, or `Ctrl-C` exits

The UI uses the core `Config` model and `Config::save_atomic`; it does not
perform text-based edits or maintain a second configuration format. Preview
rendering therefore has the same validation and template behavior as the
daemon and CLI.

Each snippet row shows its matching mode, tags, and whether it is command-backed.
Word-boundary snippets wait until a trailing boundary is observed, preventing
accidental expansion inside larger words. Command-backed snippets run their
configured direct program when previewed or matched; see the command security
limits in the operations guide.

If the daemon is not running, editing and preview still work. Pause/resume is
reported in the status area as unavailable until the user session socket is
reachable.

The graphical frontend provides the same snippet workflow in a native window:
search, select, edit replacement/description/tags, choose matching mode,
toggle enablement, preview, create, delete, undo, reload, and pause/resume.
It marks drafts with unsaved changes and asks whether to save, discard, or
cancel before switching to another snippet.
Triggers are editable in the graphical editor, so a newly created snippet can
be renamed without touching TOML. Empty, duplicate, oversized, or otherwise
invalid triggers are rejected by the same core validation used by the daemon.
`Duplicate` copies the selected snippet, including tags, match mode, templates,
and command settings, then assigns a collision-free trigger for quick editing.
It can also create and edit bounded direct-program expansions: enter the
program, one argument per line, timeout, and optional successful-output cache
duration. Shell syntax is never interpreted by this editor. Invalid command
settings or unsafe limits are rejected before the file is changed.
The `Template variables` palette inserts supported built-ins such as date,
time, hostname, username, newline, and tab directly into the replacement;
unknown template syntax is still rejected by core validation.
The preview input is editable, allowing a trigger to be tested inside larger
text and making word-boundary behavior visible before saving.

The `Settings` window exposes the bounded matcher buffer limit (1–4096
characters). Changes use the same validation, atomic save, undo history, and
daemon reload path as snippet edits.

The `Diagnostics` window reports daemon control-socket connectivity and the
currently discoverable input/output backends, including permission and
Wayland-session hints. When a Wayland session is active, its refresh action
also performs non-mutating input-method-v2 and wlroots virtual-keyboard
protocol probes. It never opens a libei portal consent prompt from passive
diagnostics.

`Import Espanso` accepts an existing Espanso YAML file, validates and previews
the converted expansion count, and reports unsupported entries before offering
an explicit “Replace current library” action. The current TOML is preserved in
the bounded undo history, and the daemon is asked to reload only after the new
file is durably saved.

It requires a Wayland-capable desktop session for window creation, but does
not require the global input-method backend to edit configuration.

## Color Packs in WayExpand GUI

WayExpand features a sophisticated color pack system with multiple themes including retro PC styles with authentic phosphor colors.

**Tip:** For enhanced authenticity with retro themes, see the [Retro fonts](#retro-fonts-for-wayexpand-themes) section below.

## Available Color Packs

### 🔵 Default
Modern minimalist design with a professional, contemporary aesthetic.
- **Dark mode**: Cool blues on dark backgrounds
- **Light mode**: Professional blues on light backgrounds
- Perfect for modern workflows and all-day use

### 🟢 Classic Green
VT220 terminal emulation style with authentic phosphor green glow.
- **Single color mode**: Bright green (RGB 0, 200, 0) on pure black
- Based on vintage CRT terminal phosphor chemistry
- Includes darker green accents and highlights
- Evokes nostalgia of 1980s-90s mainframe terminals

### 🟠 Classic Amber
Vintage Apple/Commodore amber monochrome monitor aesthetic.
- **Single color mode**: Warm amber (RGB 255, 191, 0) on black
- Inspired by classic 8-bit computer era displays
- Warm, eye-friendly color palette
- Historically accurate to 1970s-80s computer screens

### ⚪ Classic White
Monochrome white-on-black classic terminal style.
- **Single color mode**: Light gray on pure black
- Simple, high-contrast design
- Reminiscent of early digital displays
- Timeless and minimal aesthetic

### ⚡ Retro 80s Neon
Vibrant neon aesthetic inspired by 1980s cyberpunk design.
- **Dark mode**: Hot magenta, cyan-green, and bright yellow on dark purple-blue
- **Light mode**: Softer neon pastels
- Inspired by neon signs and retro synthwave aesthetics
- Energetic and fun for creative work

### ⚫ High Contrast
Maximum contrast for accessibility and clarity.
- **Dark mode**: Pure white on pure black
- **Light mode**: Pure black on white
- Highest possible visual distinction
- Recommended for users with visual sensitivity

## Using Color Packs

### Switching in the GUI

1. Open `wayexpand-gui`
2. Click the **🎨 Theme** button in the toolbar
3. Select your preferred color pack from the list
4. The colors update instantly

The selected color pack persists only during the current session. To make a color pack default, you would need to modify the initialization code.

## Color Pack Features

All color packs include proper handling of:

- ✅ Status indicators (enabled/disabled snippets)
- ✅ Semantic colors (success, warning, danger)
- ✅ Text contrast (maintains WCAG compliance where possible)
- ✅ Hover states and interactive feedback
- ✅ Dark and light mode variants (except monochrome packs)

### Monochrome Packs

The **Classic Green**, **Classic Amber**, and **Classic White** packs are designed as authentic monochrome displays:

- Single accent color throughout the interface
- Muted secondary colors for distinction
- Complementary warning/danger colors for visibility
- Historical accuracy to original display technology

These packs ignore dark/light mode toggles and maintain their authentic appearance.

## Technical Details

### Implementation

Color packs are defined in `crates/gui/src/colorpack.rs`:

```rust
pub enum ColorPack {
    Default,
    ClassicGreen,
    ClassicAmber,
    ClassicWhite,
    Retro80sNeon,
    HighContrast,
}
```

Each pack provides a `ColorScheme` with these properties:

```rust
pub struct ColorScheme {
    pub accent: Color32,           // Primary brand color
    pub accent_weak: Color32,      // Muted accent
    pub accent_text: Color32,      // Text on accent
    pub success: Color32,          // Green feedback
    pub warning: Color32,          // Yellow/orange warnings
    pub danger: Color32,           // Red errors
    pub muted: Color32,            // Secondary text
    pub border: Color32,           // UI borders
    pub surface: Color32,          // Panel backgrounds
    pub surface_hover: Color32,    // Hover states
    pub background: Color32,       // Main background
    pub extreme_bg: Color32,       // Code/monospace area
}
```

### Adding New Color Packs

To add a new color pack:

1. Add a variant to the `ColorPack` enum
2. Implement the pack in `ColorScheme` (add a `fn new_pack_name()` method)
3. Add the pack to the `all()` method
4. Update the colorpack selector dialog in `main.rs`

Example:

```rust
impl ColorScheme {
    fn solarized_dark() -> Self {
        Self {
            accent: Color32::from_rgb(0x26, 0x8B, 0xD2), // Solarized blue
            // ... other colors
        }
    }
}
```

## Color Authenticity

### Classic Green
The bright green (RGB 0, 200, 0) is based on the standard P4 phosphor used in VT220 terminals:
- CIE 1931 coordinates: x=0.29, y=0.60
- Perceived as "bright green" with slight yellow-green tint
- Caused less eye strain than pure green (0, 255, 0)
- Widely used in 1980s-90s mainframe environments

### Classic Amber
The warm amber (RGB 255, 191, 0) matches vintage monochrome displays:
- Used in Apple IIc, Commodore 64 amber monitors
- Warmer than "orange" but cooler than "gold"
- Required less power than bright white displays
- Historically easier on the eyes for extended viewing

## Accessibility Notes

- **High Contrast** pack: Meets WCAG AA+ standards for all text
- **Default** pack: Meets WCAG AA standards
- **Monochrome packs**: Meet WCAG AA standards with careful color choices
- **Retro 80s Neon**: May not meet accessibility standards; use High Contrast for required compliance

## Future Enhancements

Potential color packs for future releases:

- [ ] **Solarized** (dark and light variants)
- [ ] **Dracula** — Popular dark theme
- [ ] **Nord** — Arctic color scheme
- [ ] **Gruvbox** — Warm retro theme
- [ ] **One Dark** — Atom editor colors
- [ ] **Custom user themes** — User-defined color configurations
- [ ] **Time-based auto-switching** — Dark at night, light during day

## Screenshots

Would love to include screenshots of each color pack, but for now you can:

1. Run `wayexpand-gui`
2. Click **🎨 Theme**
3. Try each pack!

Enjoy your retro computing aesthetic! 🎨

## Retro Fonts for WayExpand Themes

The retro color themes (Classic Green, Classic Amber, Classic White, Terminal Blue, Commodore 64) benefit greatly from authentic period-appropriate fonts. While WayExpand uses system fonts by default, this guide shows how to install and use retro fonts to enhance theme authenticity.

## Recommended Fonts by Theme

### Classic Green (VT220 Terminal)
**Best match:** Courier New, Courier, or Courier 10 Pitch (monospace)
- CRT-era terminals used fixed-width fonts exclusively
- Install: Usually pre-installed; fallback to system default monospace
- Alt option: Liberation Mono (similar to Courier)

### Classic Amber (Vintage Monitor)
**Best match:** Courier New, Courier, or OCR-A
- Amber displays were common on 1980s word processors and terminals
- Install: Courier New is standard; OCR-A available from Google Fonts
- Alt option: IBM Courier (if available)

### Classic White (Monochrome Monitor)
**Best match:** Courier New or Courier
- White monochrome displays paired with Courier-style fonts
- Install: Usually pre-installed
- Alt option: Courier Prime (available from Google Fonts)

### Terminal Blue (IBM 3270 Mainframe)
**Best match:** IBM Courier, Courier New, or monospace
- IBM 3270 terminals used Courier-like fonts
- Install: Courier New; IBM Courier from Google Fonts
- Alt option: Courier Prime or Liberation Mono

### Commodore 64 (1982 Home Computer)
**Best match:** Courier, or C64 Truetype fonts
- C64 used a unique bitmap font, but Courier approximates the era
- Install: Courier New
- Alt option: "c64_pro" or "commodore64" fonts (from independent font sites)
- Modern: PragmaticaC64 (available on GitHub)

## Installing System Fonts (Linux)

### Ubuntu/Debian
```bash
# Courier (usually pre-installed, but ensure it's available)
sudo apt install fonts-liberation    # For Liberation Mono
sudo apt install fonts-noto-mono     # For Noto Mono
sudo apt install fonts-dejavu        # For DejaVu Sans Mono

# OCR-A for Amber theme (if desired)
sudo apt install fonts-ocraext
```

### Fedora/RHEL/CentOS
```bash
sudo dnf install liberation-fonts
sudo dnf install google-noto-mono-fonts
```

### Arch/Manjaro
```bash
pacman -S ttf-liberation
pacman -S noto-fonts-mono
```

### openSUSE
```bash
sudo zypper install liberation-fonts
sudo zypper install google-noto-mono-fonts
```

## Using Fonts with WayExpand

**Current limitation:** WayExpand uses your system's default monospace and proportional fonts. Per-theme font selection is not implemented in the current release.

**To use retro fonts with WayExpand:**

1. **KDE Plasma (Recommended):**
   - Settings → Appearance → Fonts
   - Change "Fixed width font" to Courier New or Liberation Mono
   - This affects WayExpand and all other applications

2. **GNOME (via gsettings):**
   ```bash
   gsettings set org.gnome.desktop.interface monospace-font-name "Courier New 10"
   # or
   gsettings set org.gnome.desktop.interface monospace-font-name "Liberation Mono 10"
   ```

3. **Manual (via fontconfig):**
   Edit `~/.config/fontconfig/fonts.conf` to set preferred monospace font globally

**Note:** Font changes apply system-wide, not just to WayExpand

## Font Pairing Recommendations

| Theme | Font | Fallback | Style |
|-------|------|----------|-------|
| **Default** | System Proportional | Sans-serif | Modern |
| **Classic Green** | Courier New | Liberation Mono | Monospace (CRT) |
| **Classic Amber** | Courier New | Liberation Mono | Monospace (Amber) |
| **Classic White** | Courier New | Liberation Mono | Monospace (Monochrome) |
| **Terminal Blue** | IBM Courier | Courier New | Monospace (Mainframe) |
| **Commodore 64** | C64 Truetype | Courier New | Bitmap-inspired |
| **Retro 80s Neon** | System Monospace | Courier | Monospace (Modern retro) |
| **High Contrast** | System Monospace | Courier | Monospace (Accessibility) |

## Font Installation Priority

For best visual accuracy, install fonts in this order:
1. **Liberation Mono** or **Courier** (essential for all retro themes)
2. **Google Noto Mono** (high-quality fallback)
3. **IBM Courier** or **Courier Prime** (theme-specific, optional)
4. **OCR-A** (Amber theme only, optional)

## Future: Per-Theme Fonts

Per-theme font support requires egui custom font loading and remains future work:

**Planned features:**
- Font selector in Settings panel
- Per-theme font configuration (e.g., Courier for Terminal Blue, Proportional for Default)
- Font preview in theme selector
- Automatic font detection (warn if selected font isn't installed)
- Custom .ttf/.otf font file support

**Technical note:** egui doesn't support per-theme FontFamily selection natively; a future implementation would need custom font loading.

## Linux Font Resources

- **Google Fonts:** https://fonts.google.com/ (Courier Prime, IBM Courier, open-source)
- **Noto Project:** https://fonts.google.com/noto (High-quality open-source fonts)
- **Liberation Fonts:** https://github.com/liberationfonts/ (Metric-compatible with MS fonts)
- **FontAwesome & community fonts via AUR** (Arch users): `yay -S courier-prime-fonts`

## Technical Notes

egui (the UI framework WayExpand uses) supports three font families:
- `Proportional` - Standard UI font (usually sans-serif)
- `Monospace` - Fixed-width font for code
- `Monospace` for code but Proportional for UI (current)

Future work may expand this to allow:
- Theme-specific font family selection
- Custom font loading from .ttf/.otf files
- Font fallback chains

## Accessibility Note

When using retro fonts, ensure adequate color contrast is maintained. WayExpand's built-in high-contrast theme overrides font styling to improve readability; this document is not a substitute for a formal WCAG audit.

## Language Support in WayExpand

WayExpand GUI now supports multiple languages with automatic detection and manual selection.

## Supported Languages

- **English** (en) — Default
- **Deutsch** (de) — German

## Using Different Languages

### GUI Language Selector

The easiest way to switch languages is to use the GUI:

1. Open `wayexpand-gui`
2. Click the **🌐 EN/DE** button in the toolbar
3. Select your preferred language
4. The UI updates immediately

### Environment Variable

Set the `LANG` environment variable to use German by default:

```bash
# Use German
export LANG=de_DE.UTF-8
wayexpand-gui

# Use English (default)
export LANG=en_US.UTF-8
wayexpand-gui
```

Or set it permanently in your shell configuration:

```bash
# ~/.bashrc or ~/.zshrc
export LANG=de_DE.UTF-8
```

## Architecture

### Translation System

The language system is implemented in `crates/gui/src/lang.rs`:

```rust
pub enum Language {
    English,
    German,
}

pub struct Strings {
    lang: Language,
}
```

### Adding New Languages

To add a new language (e.g., French):

1. Add the language variant to the `Language` enum:
```rust
pub enum Language {
    English,
    German,
    French,
}
```

2. Update `Language::from_env()` to detect it:
```rust
pub fn from_env() -> Self {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| {
            if lang.starts_with("fr") {
                Some(Language::French)
            } else if lang.starts_with("de") {
                Some(Language::German)
            } else {
                None
            }
        })
        .unwrap_or(Language::English)
}
```

3. Add translations to each method in `Strings`. For example:
```rust
pub fn ready(&self) -> &'static str {
    match self.lang {
        Language::English => "Ready",
        Language::German => "Fertig",
        Language::French => "Prêt",
    }
}
```

4. Update the language selector dialog in `main.rs`:
```rust
if ui.selectable_label(self.language == Language::French, "Français").clicked() {
    self.language = Language::French;
    self.strings.set_language(Language::French);
}
```

## Translation Coverage

### Implemented

The following UI elements have been translated:

- ✅ Toolbar (title, buttons, search placeholder)
- ✅ Sidebar (snippets list, empty states, filters)
- ✅ Editor (form labels, descriptions, tooltips)
- ✅ Dialogs (diagnostics, settings, import, language selector)
- ✅ Status messages (success, error, action confirmations)
- ✅ Buttons and labels (all interactive elements)

### Not Yet Translated

- Hardcoded error messages from the core library (intentionally in English for debugging)
- System messages from Wayland protocol probes
- Daemon status output (from the backend daemons)

These are intentionally left in English as they contain technical diagnostic information.

## German README

A comprehensive German README is available at `README.de.md`.

To link from other documentation:

```markdown
- English: [README.md](../README.md)
- Deutsch: [README.de.md](../README.de.md)
```

## Testing

To test language switching:

1. Build the GUI:
```bash
cargo build -p wayexpand-gui --release
```

2. Run with English:
```bash
LANG=en_US.UTF-8 ./target/release/wayexpand-gui
```

3. Run with German (via environment):
```bash
LANG=de_DE.UTF-8 ./target/release/wayexpand-gui
```

4. Run and switch in-app:
```bash
./target/release/wayexpand-gui
# Click 🌐 EN/DE button to switch languages
```

## Future Enhancements

- [ ] Add more languages (French, Spanish, Japanese, etc.)
- [ ] Support for right-to-left languages (Arabic, Hebrew)
- [ ] Community translation crowdsourcing
- [ ] Translation memory for consistency

## Contributing Translations

Want to add your language? Please:

1. Open an issue or discussion on GitHub
2. Translate all strings in the `Strings` struct
3. Test with the GUI
4. Submit a PR with the changes

Thank you for helping make WayExpand accessible in your language!

## GUI Performance Analysis & Optimization Roadmap

## Overview

This document analyzes known performance bottlenecks in the WayExpand GUI
and tracks fixes for them.

---

## Resolved

### "Use current app" Button Could Freeze the GUI Indefinitely

**Original symptom:** clicking "Use current app" in the Snippet Editor made
the GUI unresponsive, expected to last 1-2 seconds (KWin script
registration delay).

**Actual severity turned out to be worse than documented here:** the
button handler called `KwinWindowTracker::new()` synchronously on the UI
thread, and while the *window-wait* step inside it was bounded to 5
seconds, the D-Bus connection and KWin script registration steps before
that had **no bound at all**. A session bus or KWin left in a bad state
(observed in practice after a prior daemon instance was forcibly killed)
could hang that call indefinitely, freezing the entire window with no way
to recover short of killing the process.

**Fix:** the whole detection (connection, script load, window-wait) now
runs on a background thread; the UI polls a channel each frame, shows a
spinner while waiting, and offers a Cancel button that stops waiting on
the thread (without joining or killing it — it's simply abandoned if it
never answers). See `GuiApp::app_detection` in `crates/gui/src/main.rs`.

The daemon's `KwinWindowTracker::probe()` had the identical unbounded-hang
shape, called synchronously in `main()` before the event loop starts;
fixed the same way (bounded to 3 seconds on a detached thread) after it
caused a real production outage — a hung probe meant the daemon never
processed a single keystroke, with no log output explaining why.

---

## Other Known Issues

None currently tracked. If a new performance bottleneck is found, add it
here with a symptom, root cause, and severity before fixing it, so the fix
can be verified against a concrete description.
