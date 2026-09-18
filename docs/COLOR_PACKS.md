# Color Packs in WayExpand GUI

WayExpand features a sophisticated color pack system with multiple themes including retro PC styles with authentic phosphor colors.

**Tip:** For enhanced authenticity with retro themes, see [RETRO_FONTS.md](RETRO_FONTS.md) for font recommendations and installation instructions. Pairing retro themes with period-appropriate fonts (like Courier for terminal themes) dramatically improves the nostalgia factor!

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
