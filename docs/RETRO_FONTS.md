# Retro Fonts for WayExpand Themes

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

Currently, WayExpand GUI uses the system's default monospace font for code display and Proportional for UI text. To use specific fonts globally:

**Via font configuration (future feature):**
A font selector will be added to Settings in v1.2, allowing per-theme font selection.

**Via system settings:**
On Wayland, change your system's default monospace or proportional font through your desktop settings (KDE Plasma: Settings → Fonts, GNOME: Settings → Appearance).

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

## Future: Font Per-Theme

WayExpand v1.2 is planned to include:
- Font selector in Settings panel
- Per-theme font configuration
- Font preview in theme selector
- Automatic font detection (warn if selected font isn't installed)

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

In v1.2, we'll expand this to allow:
- Theme-specific font family selection
- Custom font loading from .ttf/.otf files
- Font fallback chains

## Accessibility Note

When using retro fonts, ensure adequate color contrast is maintained. WayExpand's built-in high-contrast theme overrides font styling to ensure readability. All retro themes in v1.1+ meet WCAG 2.1 AA contrast standards.
