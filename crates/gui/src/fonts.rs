//! Validated system font fallbacks for symbols and international text.
//!
//! egui's bundled fonts cover the common Latin UI, but desktop configurations
//! vary widely in their coverage of arrows, status marks, and CJK text. The
//! first valid font in each platform-specific fallback group is appended
//! behind egui's bundled fonts so the normal appearance remains unchanged.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::path::{Path, PathBuf};

const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

struct FallbackGroup {
    name: &'static str,
    candidates: Vec<(PathBuf, u32)>,
}

fn fallback_groups() -> Vec<FallbackGroup> {
    #[cfg(target_os = "windows")]
    let (symbols, cjk): (&[&str], &[&str]) = (
        &["seguisym.ttf", "segoeui.ttf"],
        &["msyh.ttc", "YuGothM.ttc", "meiryo.ttc", "malgun.ttf"],
    );
    #[cfg(target_os = "macos")]
    let (symbols, cjk): (&[&str], &[&str]) = (
        &[
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ],
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ],
    );
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (symbols, cjk): (&[&str], &[&str]) = (
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        ],
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        ],
    );

    let resolve = |path: &&str| (platform_font_path(path), 0);
    vec![
        FallbackGroup {
            name: "wayexpand-system-symbols",
            candidates: symbols.iter().map(resolve).collect(),
        },
        FallbackGroup {
            name: "wayexpand-system-cjk",
            candidates: cjk.iter().map(resolve).collect(),
        },
    ]
}

#[cfg(target_os = "windows")]
fn platform_font_path(name: &str) -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts")
        .join(name)
}

#[cfg(not(target_os = "windows"))]
fn platform_font_path(name: &str) -> PathBuf {
    PathBuf::from(name)
}

fn load_font(path: &Path, index: u32) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FONT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), index).ok()?;
    Some(bytes)
}

fn definitions_from(fallbacks: Vec<(&'static str, Vec<u8>, u32)>) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    for (name, bytes, index) in fallbacks {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        definitions
            .font_data
            .insert(name.to_owned(), std::sync::Arc::new(data));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .push(name.to_owned());
        }
    }
    definitions
}

pub(crate) fn install(ctx: &egui::Context) {
    let fallbacks = fallback_groups()
        .into_iter()
        .filter_map(|group| {
            group.candidates.iter().find_map(|(path, index)| {
                load_font(path, *index).map(|bytes| (group.name, bytes, *index))
            })
        })
        .collect();
    ctx.set_fonts(definitions_from(fallbacks));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallbacks_follow_bundled_fonts() {
        let defaults = FontDefinitions::default();
        let bundled = defaults.families[&FontFamily::Proportional].clone();
        let definitions = definitions_from(vec![("symbols", Vec::new(), 0)]);
        let proportional = &definitions.families[&FontFamily::Proportional];
        assert_eq!(&proportional[..bundled.len()], bundled.as_slice());
        assert_eq!(proportional.last().map(String::as_str), Some("symbols"));
    }

    #[test]
    fn invalid_and_missing_fonts_are_skipped() {
        assert!(ab_glyph::FontVec::try_from_vec_and_index(b"not a font".to_vec(), 0).is_err());
        assert!(load_font(Path::new("/nonexistent/font.ttf"), 0).is_none());
    }

    #[test]
    fn discovered_fonts_can_be_installed_and_rendered() {
        let context = egui::Context::default();
        install(&context);
        let _ = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("→ ✓ ● ◌ ⊘ 郵件 受信箱");
            });
        });
    }
}
