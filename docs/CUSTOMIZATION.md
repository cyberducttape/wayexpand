# GUI Customization Guide

WayExpand GUI offers extensive customization options including language support and color packs with retro PC aesthetics.

## Quick Start

### Languages 🌐

1. Click **🌐 EN/DE** in the toolbar
2. Choose **English** or **Deutsch**
3. UI updates instantly

Supported languages:
- 🇬🇧 English
- 🇩🇪 Deutsch (German)

### Color Packs 🎨

1. Click **🎨 Theme** in the toolbar
2. Choose from 6 color packs:
   - **Default** — Modern minimalist
   - **Classic Green** — VT220 CRT terminal
   - **Classic Amber** — Vintage Apple monitor
   - **Classic White** — Monochrome classic
   - **Retro 80s Neon** — Vibrant cyberpunk
   - **High Contrast** — Accessibility-focused
3. Colors update instantly

## Languages

### English
Default language with full documentation and support.

**Set as default:**
```bash
export LANG=en_US.UTF-8
wayexpand-gui
```

### Deutsch (German)
Complete German translation of all UI elements and documentation.

**Set as default:**
```bash
export LANG=de_DE.UTF-8
wayexpand-gui
```

See [GUI.md](GUI.md) for language support and adding more languages.

## Color Packs

### Modern Theme
**Default** — Professional, contemporary design
- Dark: Cool blues on dark backgrounds
- Light: Professional blues on light
- Best for: Modern workflows

### Retro PC Themes

These authentically recreate vintage computing aesthetics:

#### 🟢 Classic Green
**VT220 Terminal** — Bright green on black
- Color: RGB(0, 200, 0) — authentic P4 phosphor green
- Era: 1980s-90s mainframe terminals
- Character: Nostalgic, eye-friendly glow effect
- Best for: Terminal enthusiasts, retro vibes

#### 🟠 Classic Amber
**Vintage Monitor** — Warm amber on black  
- Color: RGB(255, 191, 0) — authentic display amber
- Era: 1970s-80s computer monitors
- Character: Warm, nostalgic, historically accurate
- Best for: Apple II / Commodore 64 fans
- Note: Easy on the eyes for extended work

#### ⚪ Classic White
**Monochrome Display** — White on black
- Color: Light gray on pure black
- Era: Classic digital displays
- Character: Simple, timeless, minimal
- Best for: Purists, high contrast preference

### Aesthetic Themes

#### ⚡ Retro 80s Neon
**Cyberpunk Aesthetic** — Vibrant neon colors
- Dark: Hot magenta, cyan-green, bright yellow
- Light: Softer neon pastels
- Era: 1980s synthwave, cyberpunk culture
- Character: Energetic, fun, visually striking
- Best for: Creative work, fun environments

#### ⚫ High Contrast
**Accessibility** — Maximum contrast
- Dark: Pure white on pure black
- Light: Pure black on white
- Standard: WCAG AA+ compliant
- Character: Maximum clarity
- Best for: Visual accessibility needs

## Implementation Details

### Files Modified

```
crates/gui/src/
├── main.rs              # Language & colorpack selectors
├── lang.rs              # NEW: Language definitions
├── colorpack.rs         # NEW: Color pack definitions
└── theme.rs             # Updated: Use ColorScheme
```

### Architecture

**Language System** (`lang.rs`):
- Language enum (English, German)
- Strings struct with 200+ translations
- Environment variable detection (LANG)
- In-app language switching

**Color Pack System** (`colorpack.rs`):
- ColorPack enum (6 themes)
- ColorScheme struct with color definitions
- Palette conversions for compatibility
- Theme installation with egui integration

## Customization

### Adding a Language

1. Add language to `Language` enum in `lang.rs`
2. Update `Language::from_env()` for detection
3. Add translations to each `Strings` method
4. Add selector button to the language dialog
5. Test with `export LANG=xx_XX.UTF-8`

### Adding a Color Pack

1. Add pack to `ColorPack` enum in `colorpack.rs`
2. Implement `ColorScheme::new_pack_name()` method
3. Add to `ColorPack::all()` list
4. Add selector item in colorpack dialog
5. Test color rendering across UI

Example new pack:

```rust
pub enum ColorPack {
    Default,
    ClassicGreen,
    // ... existing packs ...
    MyCustomPack,  // Add here
}

impl ColorScheme {
    fn my_custom_pack() -> Self {
        Self {
            accent: Color32::from_rgb(0xAA, 0xBB, 0xCC),
            // ... other colors ...
        }
    }
}
```

## Settings Persistence

**Current behavior:**
- Language and color pack selections persist during the session
- Preferences reset when closing the app

**Future enhancement:**
- Save preferences to `~/.config/wayexpand/gui.toml`
- Load on startup
- Toggle through keyboard shortcuts

## Keyboard Shortcuts

Currently supported:
- `Ctrl+S` — Save snippet
- `Ctrl+N` — New snippet
- `Esc` — Close dialogs

**Future keyboard shortcuts for customization:**
- `Ctrl+L` — Language selector
- `Ctrl+T` — Theme selector
- `Ctrl+D` — Toggle dark/light mode

## Accessibility

### Color Pack Compliance

| Pack | WCAG AA | WCAG AAA | Best For |
|------|---------|---------|----------|
| Default | ✅ | ⚠️ | General use |
| Classic Green | ✅ | ✅ | Retro fans |
| Classic Amber | ✅ | ✅ | Warm preference |
| Classic White | ✅ | ✅ | High contrast |
| Retro 80s Neon | ⚠️ | ❌ | Fun/creative |
| High Contrast | ✅ | ✅ | **A11y focus** |

### Language Support

All UI elements translated for selected language:
- Menu items
- Button labels
- Tooltips
- Dialog titles
- Status messages
- Error messages

**Not translated (intentionally English):**
- Daemon diagnostics (technical)
- Wayland protocol names (standard)
- Error codes (for debugging)

## Tips & Tricks

### Classic Green Theme
- Wear sunglasses for authentic CRT glow effect 😎
- Works great in dark environments
- Historically accurate for mainframe nostalgia

### Classic Amber Theme
- Warm on the eyes for all-day use
- Pairs well with vintage aesthetics
- Best for morning/afternoon work

### Retro 80s Neon
- Turn off lights for full cyberpunk immersion
- Great for creative sessions
- Fun for themed screenshots

### High Contrast
- Essential for accessibility needs
- Excellent for presentations
- Works in high ambient light

## Troubleshooting

**Unsupported language?**
- Language defaults to English if not detected
- Check `export LANG` to verify detection
- See [GUI.md](GUI.md)

**Wrong color pack applied?**
- Theme selector stores only current session
- Reopen GUI to get default theme
- Select desired pack after opening GUI

**Colors look wrong?**
- Check if dark/light mode matches preference
- Some packs are intentionally monochrome
- Try "Default" theme to verify settings work

## Community Contributions

Want to add your language or color pack?

1. **Languages and GUI behavior**: See [GUI.md](GUI.md)
2. **Color Packs and fonts**: See [GUI.md](GUI.md)
3. **Submit**: Open a GitHub issue or PR

Contributions are welcome! 🎨

---

**Related Documentation:**
- [GUI.md](GUI.md) — GUI, language, color pack, and font details
- [README.de.md](../README.de.md) — German documentation
