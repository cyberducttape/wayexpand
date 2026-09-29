//! Visual theme: color palette, egui `Style` construction, and the small set
//! of custom-painted widgets (snippet rows, pills, section headers) that
//! `selectable_label`/`label` alone cannot express.

use crate::colorpack::{contrast_ratio, ColorPack, ColorScheme, WCAG_AA_NORMAL_TEXT};
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Sense, Shadow, Stroke, TextStyle, Vec2,
};
use wayexpand_core::FontScale;

#[derive(Clone, Copy)]
pub struct Palette {
    pub accent: Color32,
    pub accent_weak: Color32,
    /// Text color with strong contrast against `accent` (fixed per palette
    /// since the two accent colors sit at different lightness).
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

impl Palette {
    pub fn for_pack(pack: ColorPack, dark: bool) -> Self {
        let scheme = ColorScheme::for_pack(pack, dark);
        Self {
            accent: scheme.accent,
            accent_weak: scheme.accent_weak,
            accent_text: scheme.accent_text,
            success: scheme.success,
            warning: scheme.warning,
            danger: scheme.danger,
            muted: scheme.muted,
            border: scheme.border,
            surface: scheme.surface,
            surface_hover: scheme.surface_hover,
            background: scheme.background,
            extreme_bg: scheme.extreme_bg,
        }
    }
}

/// Builds and installs a refined `Style` for both themes on top of egui's
/// own dark/light baselines, rather than replacing them wholesale: this
/// keeps every widget (color pickers, sliders, ...) that the app does not
/// explicitly restyle looking correct instead of falling back to mismatched
/// defaults. Called once at startup for both themes so switching between
/// them (via the toolbar toggle, which just flips `Context::set_theme`) is
/// instant.
pub fn install_pack(ctx: &egui::Context, pack: ColorPack, font_scale: FontScale) {
    apply_for_pack(ctx, egui::Theme::Dark, pack, font_scale);
    apply_for_pack(ctx, egui::Theme::Light, pack, font_scale);
}

/// A consistent raised surface for diagnostic blocks and modal content.
/// Keeping the frame treatment here makes custom color packs and focus
/// contrast apply uniformly instead of falling back to egui's default group.
pub fn card<R>(
    ui: &mut egui::Ui,
    palette: &Palette,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(palette.surface_hover)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, add_contents)
}

fn apply_for_pack(ctx: &egui::Context, theme: egui::Theme, pack: ColorPack, font_scale: FontScale) {
    let dark = theme == egui::Theme::Dark;
    let palette = Palette::for_pack(pack, dark);
    let mut style = (*ctx.style_of(theme)).clone();
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let scale = font_scale.multiplier();

    visuals.override_text_color = None;
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_weak;
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.window_fill = palette.surface;
    visuals.panel_fill = palette.background;
    visuals.faint_bg_color = palette.surface_hover;
    visuals.extreme_bg_color = palette.extreme_bg;
    visuals.code_bg_color = visuals.extreme_bg_color;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.danger;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0, palette.border);

    // Enhanced shadow for depth and polish
    visuals.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 50 }),
    };
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.popup_shadow = visuals.window_shadow;

    for widgets in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widgets.corner_radius = CornerRadius::same(8);
    }

    // Keep standard egui controls on the same quiet, layered surface system
    // as the custom cards. The default egui fills are especially conspicuous
    // beside the sidebar/editor cards and make the interface feel assembled
    // from unrelated widgets.
    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.weak_bg_fill = palette.surface;
    visuals.widgets.hovered.bg_fill = palette.surface_hover;
    visuals.widgets.hovered.weak_bg_fill = palette.surface_hover;
    visuals.widgets.active.bg_fill = palette.accent_weak;
    visuals.widgets.active.weak_bg_fill = palette.accent_weak;
    visuals.widgets.open.bg_fill = palette.surface_hover;
    visuals.widgets.open.weak_bg_fill = palette.surface_hover;

    // Keyboard focus indicator: dashed border for accessibility
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(2.0, palette.border);

    // Enhanced hover state: stronger accent color + subtle shadow effect
    visuals.widgets.hovered.bg_stroke = Stroke::new(2.0, palette.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(2.0, palette.accent);

    // Active/focused state: brightest indicator for keyboard users
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(2.5, palette.accent);

    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);

    // Improved text input field sizing for better readability and editing
    style.spacing.text_edit_width = f32::INFINITY; // Use full available width

    style.spacing.window_margin = Margin::same(18);
    style.spacing.menu_margin = Margin::same(10);
    style.spacing.indent = 20.0;
    style.spacing.icon_width = 16.0;
    style.spacing.scroll.bar_width = 8.0;

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(21.0 * scale, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(14.5 * scale, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(14.5 * scale, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0 * scale, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(14.0 * scale, FontFamily::Monospace),
        ),
    ]
    .into();

    ctx.set_style_of(theme, style);
}

/// A bold label used above a group of related fields.
/// Typography hierarchy: large, strong title with visual weight.
pub fn section_header(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        if !icon.is_empty() {
            ui.label(egui::RichText::new(icon).size(16.0).strong());
        }
        ui.label(
            egui::RichText::new(title)
                .size(16.0)
                .strong()
                .text_style(egui::TextStyle::Heading),
        );
    });
    ui.add_space(4.0); // Extra space after section header
}

/// A small rounded, colored badge (snippet/hotkey counts, status words).
pub fn pill(ui: &mut egui::Ui, text: impl Into<String>, fg: Color32, bg: Color32) {
    let background = blend_over(bg, ui.visuals().panel_fill);
    let fg = readable_text_color(fg, background);
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(fg).size(12.0).strong());
        });
}

fn blend_over(foreground: Color32, background: Color32) -> Color32 {
    let alpha = f32::from(foreground.a()) / 255.0;
    let channel = |foreground: u8, background: u8| {
        (f32::from(foreground) * alpha + f32::from(background) * (1.0 - alpha)).round() as u8
    };
    Color32::from_rgb(
        channel(foreground.r(), background.r()),
        channel(foreground.g(), background.g()),
        channel(foreground.b(), background.b()),
    )
}

fn readable_text_color(preferred: Color32, background: Color32) -> Color32 {
    if contrast_ratio(preferred, background) >= WCAG_AA_NORMAL_TEXT {
        return preferred;
    }
    let black_ratio = contrast_ratio(Color32::BLACK, background);
    let white_ratio = contrast_ratio(Color32::WHITE, background);
    if black_ratio >= white_ratio {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// A small, clickable rounded chip used for the category filter row --
/// visually similar to `pill` but interactive, filling solid with the accent
/// color when `selected`. All metrics (font, padding, height) scale by
/// `scale` so the user's font-scale accessibility setting reaches this
/// custom-painted widget, not just standard text.
pub fn chip_scaled(
    ui: &mut egui::Ui,
    palette: &Palette,
    text: &str,
    selected: bool,
    scale: f32,
) -> egui::Response {
    let font = FontId::new(12.0 * scale, FontFamily::Proportional);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::TRANSPARENT);
    let size = Vec2::new(galley.size().x + 20.0 * scale, 24.0 * scale);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (fg, bg) = if selected {
            (palette.accent_text, palette.accent)
        } else if hovered {
            (ui.visuals().text_color(), palette.surface_hover)
        } else {
            (palette.muted, tint(palette.border, 140))
        };
        let fg = readable_text_color(fg, blend_over(bg, ui.visuals().panel_fill));
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(255), bg);
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, fg);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A compact theme picker card. The swatch previews the selected pack while
/// the card itself keeps the same surface, border, and focus treatment as the
/// rest of the application.
pub fn colorpack_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    name: &str,
    description: &str,
    swatch: Color32,
    selected: bool,
    scale: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 62.0 * scale),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let background = if selected {
            palette.accent_weak
        } else if hovered {
            palette.surface_hover
        } else {
            palette.surface
        };
        let border = if selected {
            palette.accent
        } else {
            palette.border
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(10), background);
        painter.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(if selected { 2.0 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        let swatch_rect = egui::Rect::from_min_size(
            rect.left_center() + Vec2::new(14.0 * scale, -16.0 * scale),
            Vec2::splat(32.0 * scale),
        );
        painter.rect_filled(swatch_rect, CornerRadius::same(8), swatch);
        painter.rect_stroke(
            swatch_rect,
            CornerRadius::same(8),
            Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
        let text_left = swatch_rect.right() + 12.0 * scale;
        painter.text(
            egui::pos2(text_left, rect.top() + 12.0 * scale),
            egui::Align2::LEFT_TOP,
            name,
            FontId::new(14.0 * scale, FontFamily::Proportional),
            ui.visuals().text_color(),
        );
        painter.text(
            egui::pos2(text_left, rect.top() + 34.0 * scale),
            egui::Align2::LEFT_TOP,
            description,
            FontId::new(11.5 * scale, FontFamily::Proportional),
            palette.muted,
        );
        if selected {
            painter.circle_filled(
                egui::pos2(rect.right() - 18.0 * scale, rect.center().y),
                5.0 * scale,
                palette.accent,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An accent-filled call-to-action button, for the one primary action in a
/// given context (Save changes, Create snippet, ...).
pub fn primary_button(ui: &mut egui::Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .color(palette.accent_text)
                .strong(),
        )
        .fill(palette.accent)
        .stroke(Stroke::NONE),
    )
}

/// The standard secondary action treatment. Keeping this centralized prevents
/// dialogs and toolbars from mixing egui's default gray buttons with the
/// application's surfaces and borders.
pub fn secondary_button(ui: &mut egui::Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(ui.visuals().text_color()))
            .fill(palette.surface_hover)
            .stroke(Stroke::new(1.0, palette.border)),
    )
}

/// An outlined, danger-colored button for destructive actions.
pub fn danger_button(ui: &mut egui::Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(palette.danger))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, palette.danger)),
    )
}

pub struct SnippetRow<'a> {
    pub selected: bool,
    pub enabled: bool,
    pub command_backed: bool,
    pub trigger: &'a str,
    pub detail: &'a str,
    pub category: &'a str,
}

/// The two independently clickable zones of a `snippet_row`: the row body
/// (select this snippet) and the small status dot (toggle enabled, without
/// touching selection or any in-progress unsaved draft).
pub struct SnippetRowResponse {
    pub row: egui::Response,
    pub toggle: egui::Response,
}

/// A custom-painted sidebar list entry: a status dot, the trigger in
/// monospace, a muted detail line, and (for command-backed snippets) a small
/// badge -- laid out as a rounded card that highlights on hover and tints
/// with the accent color when selected. The row height, offsets, and font
/// sizes all scale by `scale` so the font-scale accessibility setting
/// (Settings > font size) actually enlarges the most-viewed list in the app,
/// not just the panels around it.
pub fn snippet_row_scaled(
    ui: &mut egui::Ui,
    palette: &Palette,
    row: SnippetRow<'_>,
    scale: f32,
) -> SnippetRowResponse {
    let width = ui.available_width();
    let height = 48.0 * scale;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    let dot_center = rect.left_center() + Vec2::new(16.0 * scale, 0.0);
    let toggle_rect = egui::Rect::from_center_size(dot_center, Vec2::splat(20.0 * scale));
    let toggle_response = ui
        .interact(toggle_rect, response.id.with("toggle"), Sense::click())
        .on_hover_text(if row.enabled {
            "Click to disable"
        } else {
            "Click to enable"
        })
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let bg = if row.selected {
            palette.accent_weak
        } else if hovered {
            palette.surface_hover
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(8), bg);
        if row.selected {
            let bar = egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
            painter.rect_filled(bar, CornerRadius::same(2), palette.accent);
        }

        let dot_color = if row.enabled {
            palette.success
        } else {
            palette.muted
        };
        if toggle_response.hovered() {
            painter.circle_stroke(dot_center, 7.0 * scale, Stroke::new(1.5, dot_color));
        }
        painter.circle_filled(dot_center, 4.0 * scale, dot_color);

        let text_left = rect.left() + 30.0 * scale;
        let trigger_pos = egui::pos2(text_left, rect.top() + 9.0 * scale);
        painter.text(
            trigger_pos,
            egui::Align2::LEFT_TOP,
            row.trigger,
            FontId::new(14.5 * scale, FontFamily::Monospace),
            ui.visuals().text_color(),
        );
        if row.command_backed {
            let galley = painter.layout_no_wrap(
                row.trigger.to_owned(),
                FontId::new(14.5 * scale, FontFamily::Monospace),
                Color32::TRANSPARENT,
            );
            let badge_font = FontId::new(10.0 * scale, FontFamily::Proportional);
            let text_galley =
                painter.layout_no_wrap("cmd".to_owned(), badge_font.clone(), Color32::TRANSPARENT);
            let pad = Vec2::new(6.0, 2.0) * scale;
            let badge_size = text_galley.size() + pad * 2.0;
            let badge_rect = egui::Rect::from_min_size(
                trigger_pos + Vec2::new(galley.size().x + 8.0 * scale, -1.0),
                badge_size,
            );
            painter.rect_filled(
                badge_rect,
                CornerRadius::same(255),
                tint(palette.warning, 34),
            );
            painter.text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                "cmd",
                badge_font,
                palette.warning,
            );
        }
        if !row.category.is_empty() {
            let category_font = FontId::new(10.5 * scale, FontFamily::Proportional);
            let galley = painter.layout_no_wrap(
                row.category.to_owned(),
                category_font.clone(),
                Color32::TRANSPARENT,
            );
            let pad = Vec2::new(8.0, 3.0) * scale;
            let chip_size = galley.size() + pad * 2.0;
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.right() - chip_size.x - 10.0 * scale,
                    rect.top() + 9.0 * scale,
                ),
                chip_size,
            );
            painter.rect_filled(chip_rect, CornerRadius::same(255), tint(palette.accent, 30));
            painter.text(
                chip_rect.center(),
                egui::Align2::CENTER_CENTER,
                row.category,
                category_font,
                palette.accent,
            );
        }
        let detail = if row.detail.is_empty() {
            "No description"
        } else {
            row.detail
        };
        painter.text(
            egui::pos2(text_left, rect.top() + 27.0 * scale),
            egui::Align2::LEFT_TOP,
            truncate(detail, 46),
            FontId::new(12.0 * scale, FontFamily::Proportional),
            palette.muted,
        );
    }

    SnippetRowResponse {
        row: response.on_hover_cursor(egui::CursorIcon::PointingHand),
        toggle: toggle_response,
    }
}

/// A translucent tint of `color`, for a badge background that should read
/// as "this color, faintly" against either theme's surface.
pub fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_pack_applies_font_scale_to_standard_text_styles() {
        let ctx = egui::Context::default();

        install_pack(&ctx, ColorPack::Default, FontScale::ExtraLarge);

        let style = ctx.style_of(egui::Theme::Dark);
        assert_eq!(style.text_styles[&TextStyle::Body].size, 14.5 * 1.5);
        assert_eq!(style.text_styles[&TextStyle::Small].size, 12.0 * 1.5);
    }

    #[test]
    fn tinted_status_pill_chooses_a_readable_foreground() {
        let palette = Palette::for_pack(ColorPack::Default, false);
        let background = blend_over(tint(palette.warning, 38), Color32::WHITE);
        let foreground = readable_text_color(palette.warning, background);

        assert_eq!(foreground, Color32::BLACK);
        assert!(contrast_ratio(foreground, background) >= WCAG_AA_NORMAL_TEXT);
    }
}
