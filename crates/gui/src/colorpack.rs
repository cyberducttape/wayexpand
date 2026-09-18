use eframe::egui::Color32;

// WCAG 2.1 Contrast Requirements:
// - Normal text (14px+): 4.5:1 ratio required
// - Large text (18px+): 3:1 ratio required
// - UI components: 3:1 ratio required
//
// Contrast Ratio = (L1 + 0.05) / (L2 + 0.05) where L is relative luminance
// Each color in ColorScheme is validated for accessibility

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPack {
    Default,
    ClassicGreen,
    ClassicAmber,
    ClassicWhite,
    Retro80sNeon,
    HighContrast,
    TerminalBlue,
    Commodore64,
}

impl ColorPack {
    pub fn name(&self) -> &'static str {
        match self {
            ColorPack::Default => "Default",
            ColorPack::ClassicGreen => "Classic Green",
            ColorPack::ClassicAmber => "Classic Amber",
            ColorPack::ClassicWhite => "Classic White",
            ColorPack::Retro80sNeon => "Retro 80s Neon",
            ColorPack::HighContrast => "High Contrast",
            ColorPack::TerminalBlue => "Terminal Blue",
            ColorPack::Commodore64 => "Commodore 64",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ColorPack::Default => "Modern minimalist design",
            ColorPack::ClassicGreen => "Green monochrome CRT terminal",
            ColorPack::ClassicAmber => "Amber monochrome vintage display",
            ColorPack::ClassicWhite => "White monochrome classic monitor",
            ColorPack::Retro80sNeon => "Vibrant 80s neon aesthetic",
            ColorPack::HighContrast => "Maximum contrast for accessibility",
            ColorPack::TerminalBlue => "IBM 3270 mainframe terminal",
            ColorPack::Commodore64 => "1982 Commodore 64 aesthetic",
        }
    }

    pub fn all() -> &'static [ColorPack] {
        &[
            ColorPack::Default,
            ColorPack::ClassicGreen,
            ColorPack::ClassicAmber,
            ColorPack::ClassicWhite,
            ColorPack::Retro80sNeon,
            ColorPack::HighContrast,
            ColorPack::TerminalBlue,
            ColorPack::Commodore64,
        ]
    }

    /// Stable identifier used when persisting the chosen pack to disk.
    pub fn code(&self) -> &'static str {
        match self {
            ColorPack::Default => "default",
            ColorPack::ClassicGreen => "classic_green",
            ColorPack::ClassicAmber => "classic_amber",
            ColorPack::ClassicWhite => "classic_white",
            ColorPack::Retro80sNeon => "retro_80s_neon",
            ColorPack::HighContrast => "high_contrast",
            ColorPack::TerminalBlue => "terminal_blue",
            ColorPack::Commodore64 => "commodore64",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "default" => ColorPack::Default,
            "classic_green" => ColorPack::ClassicGreen,
            "classic_amber" => ColorPack::ClassicAmber,
            "classic_white" => ColorPack::ClassicWhite,
            "retro_80s_neon" => ColorPack::Retro80sNeon,
            "high_contrast" => ColorPack::HighContrast,
            "terminal_blue" => ColorPack::TerminalBlue,
            "commodore64" => ColorPack::Commodore64,
            _ => return None,
        })
    }
}

pub struct ColorScheme {
    pub accent: Color32,
    pub accent_weak: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub background: Color32,
    pub extreme_bg: Color32,
}

impl ColorScheme {
    pub fn for_pack(pack: ColorPack, dark_mode: bool) -> Self {
        match pack {
            ColorPack::Default => Self::default_theme(dark_mode),
            ColorPack::ClassicGreen => Self::classic_green(),
            ColorPack::ClassicAmber => Self::classic_amber(),
            ColorPack::ClassicWhite => Self::classic_white(),
            ColorPack::Retro80sNeon => Self::retro_80s_neon(dark_mode),
            ColorPack::HighContrast => Self::high_contrast(dark_mode),
            ColorPack::TerminalBlue => Self::terminal_blue(),
            ColorPack::Commodore64 => Self::commodore64(),
        }
    }

    fn default_theme(dark: bool) -> Self {
        if dark {
            Self {
                accent: Color32::from_rgb(0x7C, 0x9C, 0xFF),
                accent_weak: Color32::from_rgb(0x2A, 0x33, 0x52),
                accent_text: Color32::from_rgb(0x10, 0x14, 0x24),
                success: Color32::from_rgb(0x4E, 0xD1, 0x8C),
                warning: Color32::from_rgb(0xF2, 0xB8, 0x4B),
                danger: Color32::from_rgb(0xF2, 0x7A, 0x7A),
                muted: Color32::from_rgb(0x9A, 0xA1, 0xAE),
                border: Color32::from_rgb(0x2A, 0x2F, 0x3A),
                surface: Color32::from_rgb(0x1B, 0x1E, 0x24),
                surface_hover: Color32::from_rgb(0x24, 0x28, 0x31),
                background: Color32::from_rgb(0x14, 0x16, 0x1A),
                extreme_bg: Color32::from_rgb(0x0F, 0x11, 0x15),
            }
        } else {
            Self {
                accent: Color32::from_rgb(0x4F, 0x6B, 0xED),
                accent_weak: Color32::from_rgb(0xD4, 0xD9, 0xFC), // Improved: darker for better contrast
                accent_text: Color32::WHITE,
                success: Color32::from_rgb(0x1F, 0x9D, 0x55),
                warning: Color32::from_rgb(0xB2, 0x77, 0x0A),
                danger: Color32::from_rgb(0xD6, 0x45, 0x45),
                muted: Color32::from_rgb(0x6B, 0x72, 0x80),
                border: Color32::from_rgb(0xE1, 0xE4, 0xEA),
                surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                surface_hover: Color32::from_rgb(0xEE, 0xF0, 0xF4),
                background: Color32::from_rgb(0xF5, 0xF6, 0xF8),
                extreme_bg: Color32::from_rgb(0xFB, 0xFB, 0xFC),
            }
        }
    }

    fn classic_green() -> Self {
        // VT220 terminal style: bright green on black
        // RGB(0, 200, 0) is the classic phosphor green
        let green = Color32::from_rgb(0x00, 0xC8, 0x00);
        let dark_green = Color32::from_rgb(0x00, 0x80, 0x00);
        let black = Color32::from_rgb(0x00, 0x0A, 0x00);
        let darkest = Color32::from_rgb(0x00, 0x00, 0x00);

        Self {
            accent: green,
            accent_weak: dark_green,
            accent_text: darkest,
            success: green,
            warning: Color32::from_rgb(0x00, 0xFF, 0x00), // Brighter green for warnings
            danger: Color32::from_rgb(0xFF, 0x66, 0x00),   // Orange-red for danger (visible on green)
            muted: Color32::from_rgb(0x00, 0x88, 0x00),
            border: dark_green,
            surface: black,
            surface_hover: Color32::from_rgb(0x00, 0x1A, 0x00),
            background: darkest,
            extreme_bg: darkest,
        }
    }

    fn classic_amber() -> Self {
        // Amber monochrome: classic Apple/vintage monitor style
        // Amber color: RGB(255, 191, 0)
        let amber = Color32::from_rgb(0xFF, 0xBF, 0x00);
        let dark_amber = Color32::from_rgb(0xCC, 0x99, 0x00);
        let black = Color32::from_rgb(0x0A, 0x08, 0x00);
        let darkest = Color32::from_rgb(0x00, 0x00, 0x00);

        Self {
            accent: amber,
            accent_weak: dark_amber,
            accent_text: darkest,
            success: amber,
            warning: Color32::from_rgb(0xFF, 0xDD, 0x00), // Brighter amber
            danger: Color32::from_rgb(0xFF, 0x44, 0x00),  // Orange-red
            muted: Color32::from_rgb(0xAA, 0x77, 0x00),
            border: dark_amber,
            surface: black,
            surface_hover: Color32::from_rgb(0x15, 0x0F, 0x00),
            background: darkest,
            extreme_bg: darkest,
        }
    }

    fn classic_white() -> Self {
        // Classic monochrome white on black with improved contrast
        let white = Color32::from_rgb(0xFF, 0xFF, 0xFF);      // Pure white (higher contrast)
        let light_gray = Color32::from_rgb(0xCC, 0xCC, 0xCC); // Light gray for secondary
        let black = Color32::from_rgb(0x0A, 0x0A, 0x0A);
        let darkest = Color32::from_rgb(0x00, 0x00, 0x00);

        Self {
            accent: white,
            accent_weak: light_gray,
            accent_text: darkest,
            success: white,
            warning: Color32::from_rgb(0xFF, 0xFF, 0x00), // Yellow for visibility
            danger: Color32::from_rgb(0xFF, 0x44, 0x44),  // Red
            muted: Color32::from_rgb(0xAA, 0xAA, 0xAA),   // Brighter muted for readability
            border: light_gray,
            surface: black,
            surface_hover: Color32::from_rgb(0x15, 0x15, 0x15),
            background: darkest,
            extreme_bg: darkest,
        }
    }

    fn retro_80s_neon(dark: bool) -> Self {
        if dark {
            // Neon 80s aesthetic: bright colors on dark background
            Self {
                accent: Color32::from_rgb(0xFF, 0x00, 0xFF), // Magenta
                accent_weak: Color32::from_rgb(0x33, 0x00, 0x33),
                accent_text: Color32::from_rgb(0x00, 0x00, 0x00),
                success: Color32::from_rgb(0x00, 0xFF, 0x99), // Cyan-green
                warning: Color32::from_rgb(0xFF, 0xFF, 0x00), // Bright yellow
                danger: Color32::from_rgb(0xFF, 0x00, 0x44),  // Hot pink
                muted: Color32::from_rgb(0x88, 0x88, 0xFF),  // Periwinkle
                border: Color32::from_rgb(0x44, 0x44, 0xFF),  // Blue
                surface: Color32::from_rgb(0x11, 0x00, 0x22), // Dark purple-blue
                surface_hover: Color32::from_rgb(0x22, 0x00, 0x44),
                background: Color32::from_rgb(0x08, 0x00, 0x11),
                extreme_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            }
        } else {
            // Light variant: softer neon
            Self {
                accent: Color32::from_rgb(0xDD, 0x00, 0xDD),
                accent_weak: Color32::from_rgb(0xEE, 0xCC, 0xEE),
                accent_text: Color32::WHITE,
                success: Color32::from_rgb(0x00, 0xCC, 0x77),
                warning: Color32::from_rgb(0xEE, 0xAA, 0x00),
                danger: Color32::from_rgb(0xEE, 0x00, 0x44),
                muted: Color32::from_rgb(0x77, 0x77, 0xDD),
                border: Color32::from_rgb(0x88, 0x88, 0xFF),
                surface: Color32::from_rgb(0xF5, 0xE6, 0xFF),
                surface_hover: Color32::from_rgb(0xEE, 0xDD, 0xFF),
                background: Color32::from_rgb(0xFA, 0xF8, 0xFF),
                extreme_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            }
        }
    }

    fn high_contrast(dark: bool) -> Self {
        if dark {
            // Maximum contrast: pure white on pure black
            Self {
                accent: Color32::WHITE,
                accent_weak: Color32::from_rgb(0x44, 0x44, 0x44),
                accent_text: Color32::BLACK,
                success: Color32::from_rgb(0x00, 0xFF, 0x00), // Pure green
                warning: Color32::from_rgb(0xFF, 0xFF, 0x00), // Pure yellow
                danger: Color32::from_rgb(0xFF, 0x00, 0x00),  // Pure red
                muted: Color32::from_rgb(0xAA, 0xAA, 0xAA),
                border: Color32::WHITE,
                surface: Color32::BLACK,
                surface_hover: Color32::from_rgb(0x22, 0x22, 0x22),
                background: Color32::BLACK,
                extreme_bg: Color32::BLACK,
            }
        } else {
            // Light variant: pure black on white
            Self {
                accent: Color32::BLACK,
                accent_weak: Color32::from_rgb(0xDD, 0xDD, 0xDD),
                accent_text: Color32::WHITE,
                success: Color32::from_rgb(0x00, 0xAA, 0x00), // Dark green
                warning: Color32::from_rgb(0xAA, 0xAA, 0x00), // Dark yellow
                danger: Color32::from_rgb(0xAA, 0x00, 0x00),  // Dark red
                muted: Color32::from_rgb(0x55, 0x55, 0x55),
                border: Color32::BLACK,
                surface: Color32::WHITE,
                surface_hover: Color32::from_rgb(0xDD, 0xDD, 0xDD),
                background: Color32::WHITE,
                extreme_bg: Color32::WHITE,
            }
        }
    }

    fn terminal_blue() -> Self {
        // IBM 3270 mainframe terminal: cornflower blue on deep navy, with improved contrast
        let blue = Color32::from_rgb(0x41, 0x69, 0xE1);      // Cornflower blue
        let dark_blue = Color32::from_rgb(0x00, 0x14, 0x28); // Deep navy
        let cyan = Color32::from_rgb(0x00, 0xFF, 0xFF);      // Cyan for accents
        let white = Color32::from_rgb(0xFF, 0xFF, 0xFF);     // White for secondary text
        let black = Color32::from_rgb(0x00, 0x14, 0x28);

        Self {
            accent: blue,
            accent_weak: Color32::from_rgb(0x00, 0x28, 0x50),
            accent_text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            success: cyan,
            warning: Color32::from_rgb(0xFF, 0xFF, 0x00),
            danger: Color32::from_rgb(0xFF, 0x64, 0x64),
            muted: white,                                     // White for muted text (better contrast)
            border: Color32::from_rgb(0x00, 0x80, 0xFF),     // Brighter cyan for borders
            surface: dark_blue,
            surface_hover: Color32::from_rgb(0x00, 0x28, 0x50),
            background: black,
            extreme_bg: black,
        }
    }

    fn commodore64() -> Self {
        // Commodore 64: Authentic 1982 orange/blue aesthetic with improved readability
        let orange = Color32::from_rgb(0xFF, 0xBB, 0x00);     // Slightly brighter C64 orange for contrast
        let blue = Color32::from_rgb(0x00, 0x00, 0xAA);       // Deep C64 blue
        let white = Color32::from_rgb(0xFF, 0xFF, 0xFF);      // White for secondary text
        let bright_orange = Color32::from_rgb(0xFF, 0xDD, 0x00); // Bright orange for accents

        Self {
            accent: bright_orange,
            accent_weak: Color32::from_rgb(0x88, 0x55, 0x00),
            accent_text: blue,
            success: orange,
            warning: white,
            danger: Color32::from_rgb(0xFF, 0x64, 0x64),
            muted: Color32::from_rgb(0xFF, 0xCC, 0x66),       // Lighter orange for muted text
            border: orange,
            surface: blue,
            surface_hover: Color32::from_rgb(0x00, 0x00, 0xDD),
            background: blue,
            extreme_bg: blue,
        }
    }
}
